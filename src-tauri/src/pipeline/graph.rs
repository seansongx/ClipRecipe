//! 滤镜图生成器。
//!
//! 维护“当前视频流 / 当前音频流”两个游标：
//! - 简单滤镜（一进一出）用 [`FilterGraph::video`] / [`FilterGraph::audio`] 追加，会被合并进同一条链；
//! - 需要多输入多输出的滤镜（overlay、concat、amix…）先用 [`FilterGraph::take_video`] 取出当前流的标签，
//!   用 [`FilterGraph::chain`] 写一条完整的链，再用 [`FilterGraph::set_video`] 把输出设为新的当前流。
//!
//! 最终由 [`FilterGraph::render`] 生成 `-filter_complex` 字符串与 `-map` 参数；
//! 没有任何滤镜时不生成 `-filter_complex`，直接映射输入流。

/// 渲染结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedGraph {
    pub filter_complex: Option<String>,
    /// `-map` 的值，如 `0:v:0` 或 `[v1]`
    pub video_map: String,
    pub audio_map: Option<String>,
    /// 音频是否经过了滤镜（决定能否直接复制音频流）
    pub audio_filtered: bool,
}

#[derive(Debug, Clone)]
struct Pad {
    label: String,
    pending: Vec<String>,
    filtered: bool,
}

impl Pad {
    fn input(spec: &str) -> Self {
        Self {
            label: spec.to_owned(),
            pending: Vec::new(),
            filtered: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterGraph {
    chains: Vec<String>,
    video: Pad,
    audio: Option<Pad>,
    counter: usize,
}

/// 输入流标签（如 `0:v:0`）含冒号；生成的标签不含。
fn is_input_spec(label: &str) -> bool {
    label.contains(':')
}

fn map_value(label: &str) -> String {
    if is_input_spec(label) {
        label.to_owned()
    } else {
        format!("[{label}]")
    }
}

impl FilterGraph {
    /// 以第 0 个输入的第一条视频流（及第一条音频流，如有）为起点。
    pub fn new(has_audio: bool) -> Self {
        Self {
            chains: Vec::new(),
            video: Pad::input("0:v:0"),
            audio: has_audio.then(|| Pad::input("0:a:0")),
            counter: 0,
        }
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }

    /// 生成一个不重复的标签，如 `label("wm")` → `wm3`。
    pub fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}{}", self.counter);
        self.counter += 1;
        label
    }

    /// 向当前视频流追加一个一进一出的滤镜，如 `fps=30`。
    pub fn video(&mut self, filter: impl Into<String>) {
        self.video.pending.push(filter.into());
        self.video.filtered = true;
    }

    /// 向当前音频流追加一个一进一出的滤镜；没有音频时忽略并返回 false。
    pub fn audio(&mut self, filter: impl Into<String>) -> bool {
        match self.audio.as_mut() {
            Some(pad) => {
                pad.pending.push(filter.into());
                pad.filtered = true;
                true
            }
            None => false,
        }
    }

    fn flush(chains: &mut Vec<String>, counter: &mut usize, pad: &mut Pad, prefix: &str) {
        if pad.pending.is_empty() {
            return;
        }
        let out = format!("{prefix}{counter}");
        *counter += 1;
        chains.push(format!("[{}]{}[{out}]", pad.label, pad.pending.join(",")));
        pad.pending.clear();
        pad.label = out;
    }

    /// 取出当前视频流标签（不带方括号），供多输入滤镜使用。之后必须调用 [`Self::set_video`]。
    pub fn take_video(&mut self) -> String {
        Self::flush(&mut self.chains, &mut self.counter, &mut self.video, "v");
        self.video.filtered = true;
        std::mem::take(&mut self.video.label)
    }

    /// 取出当前音频流标签；没有音频时返回 None。之后应调用 [`Self::set_audio`]。
    pub fn take_audio(&mut self) -> Option<String> {
        let pad = self.audio.as_mut()?;
        Self::flush(&mut self.chains, &mut self.counter, pad, "a");
        pad.filtered = true;
        Some(std::mem::take(&mut pad.label))
    }

    /// 设置当前视频流（标签不带方括号）。
    pub fn set_video(&mut self, label: impl Into<String>) {
        self.video.label = label.into();
        self.video.filtered = true;
    }

    /// 设置当前音频流；`None` 表示输出不带音频。
    /// 原本没有音频时也可以设置（例如加背景音），此时音频视为经过滤镜。
    pub fn set_audio(&mut self, label: Option<String>) {
        self.audio = label.map(|label| Pad {
            label,
            pending: Vec::new(),
            filtered: true,
        });
    }

    /// 丢弃当前音频：已有的音频链接入 `anullsink`（避免出现未连接的滤镜输出），之后输出不带音频
    /// （之后仍可用 [`Self::set_audio`] 设置新的音频）。没有音频时什么也不做。
    pub fn discard_audio(&mut self) {
        if let Some(label) = self.take_audio() {
            self.chains.push(format!("[{label}]anullsink"));
        }
        self.audio = None;
    }

    /// 追加一条完整的链：`[in1][in2]filters[out1][out2]`。标签不带方括号。
    pub fn chain(&mut self, inputs: &[&str], filters: &str, outputs: &[&str]) {
        let ins: String = inputs.iter().map(|l| format!("[{l}]")).collect();
        let outs: String = outputs.iter().map(|l| format!("[{l}]")).collect();
        self.chains.push(format!("{ins}{filters}{outs}"));
    }

    /// 把之前追加的占位滤镜文本 `marker` 换成 `with`（流水线级步骤在模块应用完后才能确定参数时使用）。
    /// 返回是否找到。
    pub fn replace_marker(&mut self, marker: &str, with: &str) -> bool {
        let mut found = false;
        let pads = std::iter::once(&mut self.video).chain(self.audio.as_mut());
        for text in self
            .chains
            .iter_mut()
            .chain(pads.flat_map(|p| p.pending.iter_mut()))
        {
            if text.contains(marker) {
                *text = text.replace(marker, with);
                found = true;
            }
        }
        found
    }

    /// 生成最终的 `-filter_complex` 与 `-map` 值。
    pub fn render(mut self) -> Result<RenderedGraph, String> {
        Self::flush(&mut self.chains, &mut self.counter, &mut self.video, "v");
        if let Some(pad) = self.audio.as_mut() {
            Self::flush(&mut self.chains, &mut self.counter, pad, "a");
        }
        if self.video.label.is_empty() {
            return Err("滤镜图错误：视频流被取出后没有重新设置".into());
        }
        if self.audio.as_ref().is_some_and(|p| p.label.is_empty()) {
            return Err("滤镜图错误：音频流被取出后没有重新设置".into());
        }
        Ok(RenderedGraph {
            filter_complex: (!self.chains.is_empty()).then(|| self.chains.join(";")),
            video_map: map_value(&self.video.label),
            audio_map: self.audio.as_ref().map(|p| map_value(&p.label)),
            audio_filtered: self.audio.as_ref().is_some_and(|p| p.filtered),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_graph_maps_inputs_directly() {
        let r = FilterGraph::new(true).render().unwrap();
        assert_eq!(r.filter_complex, None);
        assert_eq!(r.video_map, "0:v:0");
        assert_eq!(r.audio_map.as_deref(), Some("0:a:0"));
        assert!(!r.audio_filtered);
    }

    #[test]
    fn markers_are_replaced_in_pending_and_flushed_chains() {
        let mut g = FilterGraph::new(false);
        g.video("@m@");
        g.video("fps=30");
        assert!(g.replace_marker("@m@", "a=1,b=2"));
        let label = g.take_video();
        g.set_video(label);
        assert!(!g.replace_marker("@x@", "y"));
        let r = g.render().unwrap();
        assert_eq!(
            r.filter_complex.as_deref(),
            Some("[0:v:0]a=1,b=2,fps=30[v0]")
        );
    }

    #[test]
    fn simple_filters_merge_into_one_chain() {
        let mut g = FilterGraph::new(false);
        g.video("scale=640:-2");
        g.video("fps=30");
        let r = g.render().unwrap();
        assert_eq!(
            r.filter_complex.as_deref(),
            Some("[0:v:0]scale=640:-2,fps=30[v0]")
        );
        assert_eq!(r.video_map, "[v0]");
        assert_eq!(r.audio_map, None);
    }

    #[test]
    fn complex_chain_between_simple_filters() {
        let mut g = FilterGraph::new(true);
        g.video("hflip");
        let main = g.take_video();
        let out = g.label("ov");
        g.chain(&[&main, "1:v"], "overlay=10:10", &[&out]);
        g.set_video(out);
        g.video("fps=25");
        g.audio("volume=0.5");
        let r = g.render().unwrap();
        assert_eq!(
            r.filter_complex.as_deref(),
            Some("[0:v:0]hflip[v0];[v0][1:v]overlay=10:10[ov1];[ov1]fps=25[v2];[0:a:0]volume=0.5[a3]")
        );
        assert_eq!(r.video_map, "[v2]");
        assert_eq!(r.audio_map.as_deref(), Some("[a3]"));
        assert!(r.audio_filtered);
    }

    #[test]
    fn audio_filter_without_audio_is_ignored() {
        let mut g = FilterGraph::new(false);
        assert!(!g.audio("volume=2"));
        assert_eq!(g.render().unwrap().filter_complex, None);
    }

    #[test]
    fn discard_audio_sinks_pending_filters() {
        let mut g = FilterGraph::new(true);
        g.audio("volume=2");
        g.discard_audio();
        assert!(!g.has_audio());
        g.discard_audio(); // 再次调用无副作用
        let r = g.render().unwrap();
        assert_eq!(
            r.filter_complex.as_deref(),
            Some("[0:a:0]volume=2[a0];[a0]anullsink")
        );
        assert_eq!(r.audio_map, None);

        let mut g = FilterGraph::new(true);
        g.discard_audio();
        g.set_audio(Some("bgm".into()));
        let r = g.render().unwrap();
        assert_eq!(r.filter_complex.as_deref(), Some("[0:a:0]anullsink"));
        assert_eq!(r.audio_map.as_deref(), Some("[bgm]"));
    }

    #[test]
    fn forgetting_to_set_video_is_an_error() {
        let mut g = FilterGraph::new(false);
        let _ = g.take_video();
        assert!(g.render().is_err());
    }
}

//! 下载器测试：进程内的本机 HTTP 服务器（std TcpListener）提供合成的小 tgz，不访问网络。
//! 明文 http 只能通过测试专用的 [`UrlPolicy::AllowLoopbackHttp`] 访问本机地址（正式构建里没有这个取值）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::*;
use crate::ffmpeg::manifest::{platform_key, ToolManifest, TARGET};

/// 安装同一时间只允许一个：测试之间串行。
fn serial() -> MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

// ---------------------------------------------------------------- 本机 HTTP 服务器

#[derive(Clone)]
struct Route {
    status: u16,
    body: Vec<u8>,
    /// 写 Content-Length（值为 `length` 或 body 长度）
    send_length: bool,
    length: Option<usize>,
    /// 每块之间的停顿（模拟慢速下载）
    chunk: usize,
    delay: Duration,
    /// 发出这么多字节后停住 `stall` 这么久再断开
    stall_after: Option<usize>,
    stall: Duration,
}

impl Route {
    fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            body: body.to_vec(),
            send_length: true,
            length: None,
            chunk: 16 * 1024,
            delay: Duration::ZERO,
            stall_after: None,
            stall: Duration::ZERO,
        }
    }

    fn status(code: u16) -> Self {
        Self {
            status: code,
            ..Self::ok(b"not here")
        }
    }
}

struct Server {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    hits: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Arc<Mutex<HashMap<String, Route>>> = Arc::default();
        let hits: Arc<Mutex<Vec<String>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (r, h, s) = (Arc::clone(&routes), Arc::clone(&hits), Arc::clone(&stop));
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let (r, h) = (Arc::clone(&r), Arc::clone(&h));
                std::thread::spawn(move || serve(stream, &r, &h));
            }
        });
        Self {
            port,
            routes,
            hits,
            stop,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn route(&self, path: &str, route: Route) -> String {
        self.routes.lock().unwrap().insert(path.to_owned(), route);
        self.url(path)
    }

    fn hits(&self) -> Vec<String> {
        self.hits.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn serve(mut stream: TcpStream, routes: &Mutex<HashMap<String, Route>>, hits: &Mutex<Vec<String>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
    }
    let path = first.split_whitespace().nth(1).unwrap_or("/").to_owned();
    hits.lock().unwrap().push(path.clone());
    let route = routes
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::status(404));
    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", route.status);
    if route.send_length {
        head.push_str(&format!(
            "Content-Length: {}\r\n",
            route.length.unwrap_or(route.body.len())
        ));
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let limit = route
        .stall_after
        .unwrap_or(route.body.len())
        .min(route.body.len());
    for piece in route.body[..limit].chunks(route.chunk.max(1)) {
        if stream.write_all(piece).is_err() {
            return;
        }
        let _ = stream.flush();
        if !route.delay.is_zero() {
            std::thread::sleep(route.delay);
        }
    }
    if route.stall_after.is_some() {
        std::thread::sleep(route.stall);
    }
}

// ---------------------------------------------------------------- 合成压缩包与清单

enum Kind<'a> {
    File(&'a [u8]),
    Dir,
    Symlink(&'a str),
    Hardlink(&'a str),
}

/// 生成 tgz。条目名原样写进 tar 头（可以构造 `../`、绝对路径等恶意条目）。
fn tgz(entries: &[(&str, Kind)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, kind) in entries {
        let mut header = tar::Header::new_gnu();
        let raw = &mut header.as_old_mut().name;
        raw[..name.len()].copy_from_slice(name.as_bytes());
        header.set_mode(0o755);
        let data: &[u8] = match kind {
            Kind::File(data) => {
                header.set_entry_type(tar::EntryType::Regular);
                data
            }
            Kind::Dir => {
                header.set_entry_type(tar::EntryType::Directory);
                &[]
            }
            Kind::Symlink(target) | Kind::Hardlink(target) => {
                header.set_entry_type(if matches!(kind, Kind::Symlink(_)) {
                    tar::EntryType::Symlink
                } else {
                    tar::EntryType::Link
                });
                let link = &mut header.as_old_mut().linkname;
                link[..target.len()].copy_from_slice(target.as_bytes());
                &[]
            }
        };
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, data).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

fn sha(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    sha256_hex(h)
}

fn fake_binary(tool: &str) -> Vec<u8> {
    format!("fake {tool} binary\n").repeat(2000).into_bytes()
}

fn bin_path(tool: &str) -> String {
    format!("bin/{tool}{EXE_SUFFIX}")
}

/// 正常的 npm 包：package.json、LICENSE、bin/<工具>。
fn package_tgz(tool: &str) -> Vec<u8> {
    let binary = fake_binary(tool);
    tgz(&[
        ("package/", Kind::Dir),
        ("package/package.json", Kind::File(b"{\"name\":\"x\"}")),
        ("package/LICENSE", Kind::File(b"GPL")),
        ("package/bin/", Kind::Dir),
        (&format!("package/{}", bin_path(tool)), Kind::File(&binary)),
    ])
}

fn entry(tool: &str, tarball: &[u8], urls: Vec<String>) -> PlatformEntry {
    let binary = fake_binary(tool);
    PlatformEntry {
        package: format!("@seankit/{tool}-test"),
        version: "1.0.0".into(),
        file: String::new(),
        size: tarball.len() as u64,
        sha256: sha(tarball),
        integrity: String::new(),
        binary: bin_path(tool),
        binary_sha256: sha(&binary),
        binary_size: binary.len() as u64,
        license: "GPL-3.0-or-later".into(),
        rust_targets: vec![TARGET.into()],
        urls,
    }
}

fn manifest(version: &str, ffmpeg: PlatformEntry, ffprobe: PlatformEntry) -> Manifest {
    let key = platform_key(std::env::consts::OS, std::env::consts::ARCH);
    let tool = |name: &str, e: PlatformEntry| ToolManifest {
        tool: name.into(),
        ffmpeg_version: version.into(),
        package_version: "1.0.0".into(),
        aliases: Default::default(),
        platforms: [(key.clone(), e)].into_iter().collect(),
    };
    let m = Manifest {
        ffmpeg_version: version.into(),
        package_version: "1.0.0".into(),
        tools: crate::ffmpeg::manifest::Tools {
            ffmpeg: tool("ffmpeg", ffmpeg),
            ffprobe: tool("ffprobe", ffprobe),
        },
    };
    m.validate(UrlPolicy::AllowLoopbackHttp).unwrap();
    m
}

struct Root(PathBuf);

impl Root {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("cr-managed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }

    /// 根目录下的全部条目名（排序）。
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Run {
    result: Result<Installed, InstallError>,
    progress: Vec<DownloadProgress>,
}

fn install_with(
    root: &Path,
    m: &Manifest,
    cancel: CancelToken,
    read_timeout: Duration,
    on_progress: impl Fn(&DownloadProgress, &CancelToken),
) -> Run {
    let installer = Installer {
        root: root.to_path_buf(),
        manifest: m,
        platform: m.current_platform().unwrap(),
        policy: UrlPolicy::AllowLoopbackHttp,
        timeouts: Timeouts {
            connect: Duration::from_secs(5),
            read: read_timeout,
        },
        cancel: cancel.clone(),
    };
    let mut progress = Vec::new();
    let result = installer.install(&mut |p| {
        on_progress(p, &cancel);
        progress.push(p.clone());
    });
    Run { result, progress }
}

fn install(root: &Path, m: &Manifest) -> Run {
    install_with(
        root,
        m,
        CancelToken::new(),
        Duration::from_secs(5),
        |_, _| {},
    )
}

/// 正常的一对 ffmpeg / ffprobe（各自一个地址）。
fn good_manifest(server: &Server, version: &str) -> Manifest {
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let ua = server.route(&format!("/{version}/ffmpeg.tgz"), Route::ok(&a));
    let ub = server.route(&format!("/{version}/ffprobe.tgz"), Route::ok(&b));
    manifest(
        version,
        entry("ffmpeg", &a, vec![ua]),
        entry("ffprobe", &b, vec![ub]),
    )
}

fn failure(run: &Run) -> String {
    match &run.result {
        Err(InstallError::Failed(e)) => e.clone(),
        other => panic!("应失败：{other:?}"),
    }
}

fn extracted(run: &Run) -> bool {
    run.progress.iter().any(|p| p.phase == Phase::Extract)
}

// ---------------------------------------------------------------- 测试

#[test]
fn successful_install_verifies_extracts_and_writes_receipt() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("ok");
    let m = good_manifest(&server, "9.0.2");
    let run = install(&root.0, &m);
    let installed = run.result.as_ref().unwrap();
    assert_eq!(installed.version, "9.0.2");
    assert_eq!(installed.dir, root.0.join("9.0.2"));
    for (tool, path) in [
        ("ffmpeg", &installed.paths.ffmpeg),
        ("ffprobe", &installed.paths.ffprobe),
    ] {
        assert_eq!(std::fs::read(path).unwrap(), fake_binary(tool));
        assert_eq!(
            path.parent().unwrap(),
            installed.dir,
            "两个工具在同一个目录"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o111,
                0o111
            );
        }
    }
    // 版本目录里只有两个可执行文件与安装记录（package.json、LICENSE 没有写出）
    let mut files: Vec<String> = std::fs::read_dir(&installed.dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut expected = vec![
        format!("ffmpeg{EXE_SUFFIX}"),
        format!("ffprobe{EXE_SUFFIX}"),
        RECEIPT_FILE.to_owned(),
    ];
    expected.sort();
    assert_eq!(files, expected);
    // 安装记录
    let receipt = &installed.receipt;
    assert_eq!(receipt.ffmpeg_version, "9.0.2");
    assert_eq!(receipt.tools.len(), 2);
    assert_eq!(receipt.tools[0].url, server.url("/9.0.2/ffmpeg.tgz"));
    assert_eq!(
        receipt.tools[0].sha256,
        m.tools.ffmpeg.platforms.values().next().unwrap().sha256
    );
    assert!(receipt.installed_at > 1_600_000_000);
    // 进度：单调、最后等于总数，依次经过下载 / 解出 / 安装
    let total = m.current_platform().unwrap().download_size();
    assert!(run.progress.iter().all(|p| p.total == total));
    assert!(run
        .progress
        .windows(2)
        .all(|w| w[0].downloaded <= w[1].downloaded));
    assert_eq!(run.progress.last().unwrap().downloaded, total);
    assert_eq!(run.progress.last().unwrap().phase, Phase::Install);
    assert!(run.progress.iter().any(|p| p.source == "127.0.0.1"));
    // 没有残留的临时目录；能被找到
    assert_eq!(root.names(), ["9.0.2"]);
    let (found, outdated) = find(&root.0, "9.0.2").unwrap();
    assert_eq!((found.paths, outdated), (installed.paths.clone(), false));
}

#[test]
fn falls_back_to_the_next_url_when_one_fails() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("fallback");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let dead = server.route("/mirror/ffmpeg.tgz", Route::status(503));
    let good = server.route("/npm/ffmpeg.tgz", Route::ok(&a));
    // ffprobe：第一个地址内容被篡改（哈希不符），第二个正常
    let mut tampered = b.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let bad = server.route("/mirror/ffprobe.tgz", Route::ok(&tampered));
    let good_b = server.route("/npm/ffprobe.tgz", Route::ok(&b));
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![dead, good.clone()]),
        entry("ffprobe", &b, vec![bad, good_b.clone()]),
    );
    let run = install(&root.0, &m);
    let installed = run.result.unwrap();
    assert_eq!(installed.receipt.tools[0].url, good);
    assert_eq!(installed.receipt.tools[1].url, good_b);
    assert_eq!(
        server.hits(),
        [
            "/mirror/ffmpeg.tgz",
            "/npm/ffmpeg.tgz",
            "/mirror/ffprobe.tgz",
            "/npm/ffprobe.tgz"
        ]
    );
}

#[test]
fn all_urls_failing_reports_each_and_leaves_nothing() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("allfail");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let m = manifest(
        "9.0.2",
        entry(
            "ffmpeg",
            &a,
            vec![server.url("/nope1"), server.url("/nope2")],
        ),
        entry("ffprobe", &b, vec![server.url("/nope3")]),
    );
    let run = install(&root.0, &m);
    let e = failure(&run);
    assert!(
        e.contains("ffmpeg 下载失败") && e.contains("全部 2 个地址"),
        "{e}"
    );
    assert_eq!(e.matches("服务器返回 404").count(), 2, "{e}");
    assert!(root.names().is_empty(), "{:?}", root.names());
}

#[test]
fn wrong_size_is_rejected() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("size");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let ub = server.route("/ffprobe.tgz", Route::ok(&b));

    // 服务器报告的大小与清单不符：不读正文
    let mut longer = a.clone();
    longer.extend_from_slice(b"extra");
    let u = server.route("/longer.tgz", Route::ok(&longer));
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![u]),
        entry("ffprobe", &b, vec![ub.clone()]),
    );
    let e = failure(&install(&root.0, &m));
    assert!(e.contains("文件大小不符"), "{e}");

    // 不报告大小、实际发送得更多：超过清单大小时立即停止
    let u = server.route(
        "/endless.tgz",
        Route {
            send_length: false,
            ..Route::ok(&longer)
        },
    );
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![u]),
        entry("ffprobe", &b, vec![ub.clone()]),
    );
    let e = failure(&install(&root.0, &m));
    assert!(e.contains("超过应有的"), "{e}");

    // 中途断开（截断）
    let u = server.route(
        "/truncated.tgz",
        Route {
            length: Some(a.len()),
            ..Route::ok(&a[..a.len() / 2])
        },
    );
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![u]),
        entry("ffprobe", &b, vec![ub]),
    );
    let e = failure(&install(&root.0, &m));
    assert!(e.contains("不完整") || e.contains("中断"), "{e}");
    assert!(root.names().is_empty(), "{:?}", root.names());
}

#[test]
fn wrong_tarball_hash_extracts_nothing() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("hash");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let mut wrong = entry("ffmpeg", &a, vec![server.route("/a.tgz", Route::ok(&a))]);
    wrong.sha256 = "0".repeat(64);
    let m = manifest(
        "9.0.2",
        wrong,
        entry("ffprobe", &b, vec![server.route("/b.tgz", Route::ok(&b))]),
    );
    let run = install(&root.0, &m);
    let e = failure(&run);
    assert!(e.contains("SHA-256 校验失败"), "{e}");
    assert!(!extracted(&run), "哈希不符时不应打开压缩包");
    assert!(root.names().is_empty(), "{:?}", root.names());
}

#[test]
fn wrong_binary_hash_is_rejected() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("binhash");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let mut wrong = entry("ffprobe", &b, vec![server.route("/b.tgz", Route::ok(&b))]);
    wrong.binary_sha256 = "f".repeat(64);
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![server.route("/a.tgz", Route::ok(&a))]),
        wrong,
    );
    let e = failure(&install(&root.0, &m));
    assert!(e.contains("可执行文件校验失败"), "{e}");
    assert!(root.names().is_empty(), "{:?}", root.names());
}

/// 恶意或异常的压缩包：哈希与清单一致（清单被换掉的情形），仍然只解出指定条目、拒绝危险条目。
#[test]
fn malicious_tarball_entries_are_rejected() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("evil");
    let binary = fake_binary("ffmpeg");
    let target = format!("package/{}", bin_path("ffmpeg"));
    let b = package_tgz("ffprobe");
    let ub = server.route("/b.tgz", Route::ok(&b));
    let outside = std::env::temp_dir().join(format!("evil-{}", std::process::id()));
    let _ = std::fs::remove_file(&outside);
    let big = vec![7u8; binary.len() + 1];
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "../ 路径穿越",
            tgz(&[
                (
                    &format!("package/../../evil-{}", std::process::id()),
                    Kind::File(b"x"),
                ),
                (&target, Kind::File(&binary)),
            ]),
            "不安全的路径",
        ),
        (
            "绝对路径",
            tgz(&[
                ("/tmp/evil", Kind::File(b"x")),
                (&target, Kind::File(&binary)),
            ]),
            "不安全的路径",
        ),
        (
            "盘符",
            tgz(&[
                ("C:/evil", Kind::File(b"x")),
                (&target, Kind::File(&binary)),
            ]),
            "不安全的路径",
        ),
        (
            "目标是符号链接",
            tgz(&[(&target, Kind::Symlink("/bin/sh"))]),
            "链接",
        ),
        (
            "其它条目是硬链接",
            tgz(&[
                ("package/x", Kind::Hardlink("/etc/passwd")),
                (&target, Kind::File(&binary)),
            ]),
            "链接",
        ),
        (
            "条目过大",
            tgz(&[
                ("package/huge.bin", Kind::File(&big)),
                (&target, Kind::File(&binary)),
            ]),
            "过大",
        ),
        (
            "重复的目标条目",
            tgz(&[
                (&target, Kind::File(&binary)),
                (&target, Kind::File(&binary)),
            ]),
            "重复",
        ),
        (
            "缺少目标条目",
            tgz(&[("package/bin/other", Kind::File(&binary))]),
            "没有",
        ),
    ];
    for (i, (what, archive, expected)) in cases.into_iter().enumerate() {
        let url = server.route(&format!("/evil{i}.tgz"), Route::ok(&archive));
        let mut e = entry("ffmpeg", &archive, vec![url]);
        e.binary_sha256 = sha(&binary);
        let m = manifest("9.0.2", e, entry("ffprobe", &b, vec![ub.clone()]));
        let message = failure(&install(&root.0, &m));
        assert!(message.contains(expected), "{what}：{message}");
        assert!(root.names().is_empty(), "{what}：{:?}", root.names());
    }
    assert!(!outside.exists(), "不应在根目录外写入文件");

    // 多余的普通文件：忽略、不写入
    let extra = tgz(&[
        ("package/README.md", Kind::File(b"readme")),
        ("package/bin/ffmpeg-helper", Kind::File(b"helper")),
        (&target, Kind::File(&binary)),
        ("package/zzz.txt", Kind::File(b"z")),
    ]);
    let url = server.route("/extra.tgz", Route::ok(&extra));
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &extra, vec![url]),
        entry("ffprobe", &b, vec![ub]),
    );
    let installed = install(&root.0, &m).result.unwrap();
    assert_eq!(std::fs::read_dir(&installed.dir).unwrap().count(), 3);
}

#[test]
fn stalled_transfer_hits_the_timeout_then_next_url_works() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("stall");
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let stalled = server.route(
        "/stall.tgz",
        Route {
            stall_after: Some(a.len() / 3),
            stall: Duration::from_secs(5),
            ..Route::ok(&a)
        },
    );
    let ub = server.route("/b.tgz", Route::ok(&b));
    // 只有卡住的地址：明确报告超时
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![stalled.clone()]),
        entry("ffprobe", &b, vec![ub.clone()]),
    );
    let started = std::time::Instant::now();
    let run = install_with(
        &root.0,
        &m,
        CancelToken::new(),
        Duration::from_millis(400),
        |_, _| {},
    );
    let e = failure(&run);
    assert!(e.contains("下载卡住") && e.contains("0.4 秒"), "{e}");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "应在超时后很快失败"
    );
    assert!(root.names().is_empty(), "{:?}", root.names());
    // 卡住后换下一个地址
    let good = server.route("/a.tgz", Route::ok(&a));
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![stalled, good.clone()]),
        entry("ffprobe", &b, vec![ub]),
    );
    let run = install_with(
        &root.0,
        &m,
        CancelToken::new(),
        Duration::from_millis(400),
        |_, _| {},
    );
    assert_eq!(run.result.unwrap().receipt.tools[0].url, good);
}

#[test]
fn cancel_mid_download_cleans_up() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("cancel");
    // 不可压缩的内容（约 256 KB），慢速发送；取消在解出之前发生，可执行文件的哈希无关紧要
    let mut state: u32 = 1;
    let noise: Vec<u8> = (0..256 * 1024)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16) as u8
        })
        .collect();
    let a = tgz(&[(
        &format!("package/{}", bin_path("ffmpeg")),
        Kind::File(&noise),
    )]);
    assert!(a.len() > 200 * 1024);
    let b = package_tgz("ffprobe");
    let slow = Route {
        chunk: 1024,
        delay: Duration::from_millis(20),
        ..Route::ok(&a)
    };
    let m = manifest(
        "9.0.2",
        entry("ffmpeg", &a, vec![server.route("/slow.tgz", slow)]),
        entry("ffprobe", &b, vec![server.route("/b.tgz", Route::ok(&b))]),
    );
    let run = install_with(
        &root.0,
        &m,
        CancelToken::new(),
        Duration::from_secs(5),
        |p, cancel| {
            if p.downloaded > 0 {
                cancel.cancel();
            }
        },
    );
    assert_eq!(run.result, Err(InstallError::Cancelled));
    assert!(run.progress.last().unwrap().downloaded < a.len() as u64);
    assert!(
        root.names().is_empty(),
        "临时文件应已删除：{:?}",
        root.names()
    );
}

#[test]
fn failed_or_cancelled_reinstall_keeps_the_previous_copy() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("keep");
    let first = install(&root.0, &good_manifest(&server, "9.0.2"))
        .result
        .unwrap();
    let before = std::fs::read_to_string(first.dir.join(RECEIPT_FILE)).unwrap();

    // 同版本重装：压缩包哈希不符
    let (a, b) = (package_tgz("ffmpeg"), package_tgz("ffprobe"));
    let mut bad = entry("ffmpeg", &a, vec![server.route("/x/a.tgz", Route::ok(&a))]);
    bad.sha256 = "1".repeat(64);
    let m = manifest(
        "9.0.2",
        bad,
        entry("ffprobe", &b, vec![server.route("/x/b.tgz", Route::ok(&b))]),
    );
    assert!(failure(&install(&root.0, &m)).contains("SHA-256"));
    // 取消
    let m = good_manifest(&server, "9.0.2");
    let run = install_with(
        &root.0,
        &m,
        CancelToken::new(),
        Duration::from_secs(5),
        |_, c| c.cancel(),
    );
    assert_eq!(run.result, Err(InstallError::Cancelled));
    // 进程在安装中途被结束：留下的临时目录不影响查找
    std::fs::create_dir_all(root.0.join(".staging-99999-1-1/9.0.2")).unwrap();

    let (found, outdated) = find(&root.0, "9.0.2").unwrap();
    assert!(!outdated);
    assert_eq!(found.paths, first.paths);
    assert_eq!(
        std::fs::read_to_string(first.dir.join(RECEIPT_FILE)).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(&first.paths.ffmpeg).unwrap(),
        fake_binary("ffmpeg")
    );

    // 成功重装同一版本：替换、清理残留（一小时内的其它进程临时目录保留）
    let again = install(&root.0, &good_manifest(&server, "9.0.2"))
        .result
        .unwrap();
    assert_eq!(again.paths, first.paths);
    assert_eq!(root.names(), [".staging-99999-1-1", "9.0.2"]);
}

#[test]
fn update_replaces_the_older_version_and_remove_all_cleans_up() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("update");
    install(&root.0, &good_manifest(&server, "8.1.0"))
        .result
        .unwrap();
    // 旧版本：仍可用，标记为可更新
    let (old, outdated) = find(&root.0, "9.0.2").unwrap();
    assert_eq!(old.version, "8.1.0");
    assert!(outdated);
    assert!(
        is_older("8.1.0", "9.0.2") && !is_older("9.0.2", "9.0.2") && !is_older("10.0", "9.0.2")
    );

    install(&root.0, &good_manifest(&server, "9.0.2"))
        .result
        .unwrap();
    assert_eq!(root.names(), ["9.0.2"], "旧版本已删除");
    let (now, outdated) = find(&root.0, "9.0.2").unwrap();
    assert_eq!((now.version.as_str(), outdated), ("9.0.2", false));

    assert_eq!(remove_all(&root.0), Ok(1));
    assert!(!root.0.exists());
    assert!(find(&root.0, "9.0.2").is_none());
    assert_eq!(remove_all(&root.0), Ok(0));
}

#[test]
fn receipt_must_match_files_on_disk() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("receipt");
    let installed = install(&root.0, &good_manifest(&server, "9.0.2"))
        .result
        .unwrap();
    assert!(read_installed(&installed.dir).is_some());
    // 文件被截断：不算完整
    std::fs::write(&installed.paths.ffprobe, b"short").unwrap();
    assert!(read_installed(&installed.dir).is_none());
    // 安装记录损坏 / 版本与目录名不一致
    std::fs::write(&installed.paths.ffprobe, fake_binary("ffprobe")).unwrap();
    assert!(read_installed(&installed.dir).is_some());
    let receipt = installed.dir.join(RECEIPT_FILE);
    let mut r = installed.receipt.clone();
    r.ffmpeg_version = "1.0".into();
    std::fs::write(&receipt, serde_json::to_string(&r).unwrap()).unwrap();
    assert!(read_installed(&installed.dir).is_none());
    std::fs::write(&receipt, "{").unwrap();
    assert!(read_installed(&installed.dir).is_none());
}

#[test]
fn https_only_policy_refuses_plain_http_without_connecting() {
    let _s = serial();
    let server = Server::start();
    let root = Root::new("https");
    let m = good_manifest(&server, "9.0.2");
    let installer = Installer {
        root: root.0.clone(),
        manifest: &m,
        platform: m.current_platform().unwrap(),
        policy: UrlPolicy::HttpsOnly,
        timeouts: Timeouts::DOWNLOAD,
        cancel: CancelToken::new(),
    };
    let e = installer.install(&mut |_| {}).unwrap_err().to_string();
    assert!(e.contains("只允许 HTTPS"), "{e}");
    assert!(server.hits().is_empty(), "不应发出任何请求");
    assert!(root.names().is_empty(), "{:?}", root.names());
}

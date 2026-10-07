//! Installing the toolchain: the pins, the network, the archives and the tree they end up in.
//!
//! The archives here are small fakes built in the test (a `node` that prints its version, a `pnpm` likewise), served
//! by a local HTTP server that can also misbehave. What is under test is the installer's checking and bookkeeping, not
//! Node. The real archives are exercised by `real_toolchain.rs`, which is opt-in because it downloads ~80 MB.
#![cfg(unix)]

use flate2::write::GzEncoder;
use flate2::Compression;
use local_app_host::{Artifact, Source, Spec, Status, ToolchainError, Toolchains, RECEIPT_FILE};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn tgz(entries: &[(&str, &[u8], u8)]) -> Vec<u8> {
    // (path, content, tar type flag: b'0' file, b'2' symlink with the content as target)
    let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::fast()));
    for (path, content, kind) in entries {
        let mut header = tar::Header::new_gnu();
        if path.contains("..") {
            // The builder refuses to write such a name, so write the header bytes directly: this is the archive a
            // hostile server would send.
            let name = &mut header.as_gnu_mut().unwrap().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
        } else {
            header.set_path(path).unwrap();
        }
        header.set_mode(0o644);
        if *kind == b'2' {
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_link_name(std::str::from_utf8(content).unwrap()).unwrap();
            header.set_cksum();
            builder.append(&header, std::io::empty()).unwrap();
        } else {
            header.set_size(content.len() as u64);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What a request for a path gets.
#[derive(Clone)]
enum Reply {
    Body(Vec<u8>),
    Status(u16),
    /// Zeros without end, until the client leaves.
    Endless,
}

struct Server {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Reply>>>,
    hits: Arc<AtomicUsize>,
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes: Arc<Mutex<HashMap<String, Reply>>> = Arc::default();
        let hits = Arc::new(AtomicUsize::new(0));
        let (r, h) = (routes.clone(), hits.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let (routes, hits) = (r.clone(), h.clone());
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 4096];
                    let n = stream.read(&mut buffer).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..n]).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                    hits.fetch_add(1, Ordering::SeqCst);
                    let reply = routes.lock().unwrap().get(&path).cloned().unwrap_or(Reply::Status(404));
                    match reply {
                        Reply::Body(body) => {
                            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(head.as_bytes()).await;
                            let _ = stream.write_all(&body).await;
                        }
                        Reply::Status(code) => {
                            let head = format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            let _ = stream.write_all(head.as_bytes()).await;
                        }
                        Reply::Endless => {
                            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").await;
                            let zeros = vec![0_u8; 64 * 1024];
                            while stream.write_all(&zeros).await.is_ok() {}
                        }
                    }
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { port, routes, hits }
    }
    fn serve(&self, path: &str, reply: Reply) {
        self.routes.lock().unwrap().insert(path.into(), reply);
    }
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    data: PathBuf,
    server: Server,
    spec: Spec,
    node_archive: Vec<u8>,
    pnpm_archive: Vec<u8>,
}

fn script(version: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho {version}\n").into_bytes()
}

fn spec_for(server: &Server, node: &[u8], pnpm: &[u8]) -> Spec {
    Spec {
        key: "pnpm@12.5.1/node@26.9.0".into(),
        node_version: "v26.9.0".into(),
        pnpm_version: "12.5.1".into(),
        node: Artifact { url: server.url("/node.tar.gz"), sha256: sha256(node), size: node.len() as u64, entry: "node-v26.9.0-x/bin/node".into() },
        pnpm: Artifact { url: server.url("/pnpm.tgz"), sha256: sha256(pnpm), size: pnpm.len() as u64, entry: "package/pnpm".into() },
    }
}

async fn fixture() -> Fixture {
    let server = Server::start().await;
    let node_archive = tgz(&[("node-v26.9.0-x/LICENSE", b"license", b'0'), ("node-v26.9.0-x/bin/node", &script("v26.9.0"), b'0'), ("node-v26.9.0-x/bin/npm", b"../lib/npm", b'2')]);
    let pnpm_archive = tgz(&[("package/package.json", b"{}", b'0'), ("package/pnpm", &script("12.5.1"), b'0')]);
    server.serve("/node.tar.gz", Reply::Body(node_archive.clone()));
    server.serve("/pnpm.tgz", Reply::Body(pnpm_archive.clone()));
    let spec = spec_for(&server, &node_archive, &pnpm_archive);
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    Fixture { _dir: dir, data, server, spec, node_archive, pnpm_archive }
}

fn quiet(_: &str) {}

fn leftovers(data: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(data.join("toolchains"))
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|n| n.starts_with(".staging"));
    names
}

#[tokio::test]
async fn an_install_downloads_checks_extracts_and_records_what_it_installed() {
    let f = fixture().await;
    let toolchains = Toolchains::in_data_root(&f.data);
    assert_eq!(toolchains.status(&f.spec).await, Status::NotInstalled);

    let receipt = toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    assert_eq!(receipt.key, "pnpm@12.5.1/node@26.9.0");
    assert_eq!((receipt.node.version.as_str(), receipt.pnpm.version.as_str()), ("v26.9.0", "12.5.1"));
    assert_eq!(receipt.node.archive_sha256, f.spec.node.sha256);
    assert_eq!(receipt.node.binary_sha256, sha256(&script("v26.9.0")));

    let dir = toolchains.dir(&f.spec);
    assert_eq!(dir, f.data.join("toolchains/pnpm@12.5.1/node@26.9.0"));
    for program in ["bin/node", "bin/pnpm"] {
        let mode = std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(dir.join(program)).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o755, "{program}");
    }
    // Only the two programs and the receipt were taken from the archives: no LICENSE, no npm link, no package.json.
    let mut tree: Vec<String> = Vec::new();
    for entry in walk(&dir) {
        tree.push(entry.strip_prefix(&dir).unwrap().to_string_lossy().into_owned());
    }
    tree.sort();
    assert_eq!(tree, ["bin/node", "bin/pnpm", RECEIPT_FILE]);
    assert!(matches!(toolchains.status(&f.spec).await, Status::Ready(r) if r == receipt));
    assert!(leftovers(&f.data).is_empty(), "{:?}", leftovers(&f.data));
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn installing_again_downloads_nothing() {
    let f = fixture().await;
    let toolchains = Toolchains::in_data_root(&f.data);
    let first = toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    let hits = f.server.hits.load(Ordering::SeqCst);
    assert_eq!(hits, 2);
    let second = toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    assert_eq!(second, first);
    assert_eq!(f.server.hits.load(Ordering::SeqCst), hits, "an intact install must not touch the network");
}

#[tokio::test]
async fn an_archive_that_is_not_the_pinned_one_is_discarded_unread() {
    let f = fixture().await;
    // Same size, different bytes: the tampered archive even contains a working `node`.
    let mut tampered = f.node_archive.clone();
    let last = tampered.len() - 5;
    tampered[last] ^= 0xff;
    f.server.serve("/node.tar.gz", Reply::Body(tampered));
    let toolchains = Toolchains::in_data_root(&f.data);
    let error = toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(matches!(&error, ToolchainError::Mismatch { what, .. } if what == "node.tar.gz"), "{error}");
    assert!(error.to_string().starts_with("toolchain_integrity:") && error.to_string().contains("nothing was installed"));
    assert_eq!(toolchains.status(&f.spec).await, Status::NotInstalled);
    assert!(!toolchains.dir(&f.spec).exists());
    assert!(leftovers(&f.data).is_empty());
}

#[tokio::test]
async fn a_download_longer_than_pinned_is_cut_off_instead_of_filling_the_disk() {
    let f = fixture().await;
    f.server.serve("/node.tar.gz", Reply::Endless);
    let toolchains = Toolchains::in_data_root(&f.data);
    let started = std::time::Instant::now();
    let error = toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(error.to_string().contains("larger than the pinned"), "{error}");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert!(leftovers(&f.data).is_empty());
}

#[tokio::test]
async fn a_short_download_is_a_mismatch_too() {
    let f = fixture().await;
    f.server.serve("/pnpm.tgz", Reply::Body(f.pnpm_archive[..f.pnpm_archive.len() - 10].to_vec()));
    let error = Toolchains::in_data_root(&f.data).install(&f.spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(matches!(error, ToolchainError::Mismatch { .. }), "{error}");
}

#[tokio::test]
async fn a_server_that_says_no_is_reported_with_its_status() {
    let f = fixture().await;
    f.server.serve("/node.tar.gz", Reply::Status(503));
    let error = Toolchains::in_data_root(&f.data).install(&f.spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(matches!(&error, ToolchainError::BadResponse { detail, .. } if detail.contains("503")), "{error}");
}

#[tokio::test]
async fn with_no_network_the_error_says_what_to_do_instead_and_nothing_is_left() {
    let f = fixture().await;
    // A port nothing listens on: bind, note the port, drop.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut spec = f.spec.clone();
    spec.node.url = format!("http://127.0.0.1:{dead}/node.tar.gz");
    let toolchains = Toolchains::in_data_root(&f.data);
    let error = toolchains.install(&spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(matches!(error, ToolchainError::Unreachable { .. }), "{error}");
    let text = error.to_string();
    assert!(text.starts_with("toolchain_unreachable:") && text.contains("--from DIR") && text.contains("node.tar.gz"), "{text}");
    assert!(!toolchains.dir(&spec).exists() && leftovers(&f.data).is_empty());
}

#[tokio::test]
async fn archives_in_a_directory_install_without_any_network_and_are_checked_the_same() {
    let f = fixture().await;
    let offline = tempfile::tempdir().unwrap();
    std::fs::write(offline.path().join("node.tar.gz"), &f.node_archive).unwrap();
    let toolchains = Toolchains::in_data_root(&f.data);

    // One archive missing: the error names the file and where it should be.
    let error = toolchains.install(&f.spec, &Source::Directory(offline.path().into()), &quiet).await.unwrap_err();
    assert!(error.to_string().contains("pnpm.tgz") && error.to_string().contains("must be in that directory"), "{error}");
    assert!(!toolchains.dir(&f.spec).exists());

    // A wrong archive under the right name is refused like a bad download.
    std::fs::write(offline.path().join("pnpm.tgz"), tgz(&[("package/pnpm", &script("12.5.1"), b'0'), ("package/extra", b"x", b'0')])).unwrap();
    let error = toolchains.install(&f.spec, &Source::Directory(offline.path().into()), &quiet).await.unwrap_err();
    assert!(matches!(error, ToolchainError::Mismatch { .. }), "{error}");

    std::fs::write(offline.path().join("pnpm.tgz"), &f.pnpm_archive).unwrap();
    f.server.serve("/node.tar.gz", Reply::Status(500));
    f.server.serve("/pnpm.tgz", Reply::Status(500));
    toolchains.install(&f.spec, &Source::Directory(offline.path().into()), &quiet).await.unwrap();
    assert_eq!(f.server.hits.load(Ordering::SeqCst), 0, "a directory install must not use the network");
}

#[tokio::test]
async fn an_archive_whose_program_is_a_link_or_missing_is_refused() {
    let f = fixture().await;
    for (label, archive) in [
        ("a symlink in place of the program", tgz(&[("node-v26.9.0-x/bin/node", b"/bin/sh", b'2')])),
        ("the program missing", tgz(&[("node-v26.9.0-x/bin/other", &script("v26.9.0"), b'0')])),
        ("a traversal path only", tgz(&[("../../node-v26.9.0-x/bin/node", &script("v26.9.0"), b'0')])),
    ] {
        f.server.serve("/node.tar.gz", Reply::Body(archive.clone()));
        let spec = spec_for(&f.server, &archive, &f.pnpm_archive);
        let toolchains = Toolchains::in_data_root(&f.data);
        let error = toolchains.install(&spec, &Source::Network, &quiet).await.unwrap_err();
        assert!(matches!(error, ToolchainError::BadArchive { .. }), "{label}: {error}");
        assert!(!toolchains.dir(&spec).exists() && leftovers(&f.data).is_empty(), "{label}");
    }
    // Nothing escaped the staging directory either.
    assert!(!f.data.parent().unwrap().join("node-v26.9.0-x").exists());
}

#[tokio::test]
async fn a_program_that_reports_another_version_is_not_installed() {
    // P1: a different Node builds the same output, so "it builds" proves nothing about the contract's toolchain.
    let f = fixture().await;
    let node = tgz(&[("node-v26.9.0-x/bin/node", &script("v25.2.1"), b'0')]);
    f.server.serve("/node.tar.gz", Reply::Body(node.clone()));
    let spec = spec_for(&f.server, &node, &f.pnpm_archive);
    let toolchains = Toolchains::in_data_root(&f.data);
    let error = toolchains.install(&spec, &Source::Network, &quiet).await.unwrap_err();
    assert!(matches!(&error, ToolchainError::WrongProgram { program, detail } if program == "node" && detail.contains("v25.2.1")), "{error}");
    assert!(!toolchains.dir(&spec).exists() && leftovers(&f.data).is_empty());
}

#[tokio::test]
async fn an_altered_install_is_reported_damaged_and_the_next_install_replaces_it() {
    let f = fixture().await;
    let toolchains = Toolchains::in_data_root(&f.data);
    toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    let node = toolchains.dir(&f.spec).join("bin/node");

    // Same version output, different bytes: only the digest notices.
    std::fs::write(&node, b"#!/bin/sh\necho v26.9.0\n# and something else\n").unwrap();
    let Status::Damaged(why) = toolchains.status(&f.spec).await else { panic!("an altered node must not be Ready") };
    assert!(why.contains("the installed node") && why.contains("hashes to"), "{why}");

    // A program that now says something else is damaged for that reason.
    std::fs::write(&node, script("v1.0.0")).unwrap();
    let Status::Damaged(_) = toolchains.status(&f.spec).await else { panic!("wrong version must not be Ready") };

    let hits = f.server.hits.load(Ordering::SeqCst);
    toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    assert!(f.server.hits.load(Ordering::SeqCst) > hits, "a damaged install is replaced, not trusted");
    assert!(matches!(toolchains.status(&f.spec).await, Status::Ready(_)));
}

#[tokio::test]
async fn a_toolchain_installed_for_other_pins_is_not_this_toolchain() {
    let f = fixture().await;
    let toolchains = Toolchains::in_data_root(&f.data);
    toolchains.install(&f.spec, &Source::Network, &quiet).await.unwrap();
    let mut newer = f.spec.clone();
    newer.node.sha256 = "0".repeat(64);
    let Status::Damaged(why) = toolchains.status(&newer).await else { panic!("a pin change must invalidate the install") };
    assert!(why.contains("other archives"), "{why}");
}

#[tokio::test]
async fn two_installs_at_once_download_once_and_both_succeed() {
    let f = fixture().await;
    let (a, b) = (Toolchains::in_data_root(&f.data), Toolchains::in_data_root(&f.data));
    let (sa, sb) = (f.spec.clone(), f.spec.clone());
    let (ra, rb) = tokio::join!(
        tokio::spawn(async move { a.install(&sa, &Source::Network, &quiet).await }),
        tokio::spawn(async move { b.install(&sb, &Source::Network, &quiet).await })
    );
    let (ra, rb) = (ra.unwrap().unwrap(), rb.unwrap().unwrap());
    assert_eq!(ra, rb);
    assert_eq!(f.server.hits.load(Ordering::SeqCst), 2, "the second install found the first one's result");
}

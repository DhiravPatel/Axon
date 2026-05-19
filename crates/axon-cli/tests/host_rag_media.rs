//! Stage 12 — `rag_*` and `media_*` bindings exercised through the binary.

use std::path::PathBuf;
use std::process::Command;

fn axon_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // workspace root
    p.push("target");
    p.push("debug");
    p.push("axon");
    p
}

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn build_axon() {
    let status = Command::new("cargo")
        .args(["build", "-q", "--bin", "axon"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo build axon");
    assert!(status.success(), "build failed");
}

fn temp_dir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    p.push(format!("axon-stage12-{name}-{pid}-{ts}"));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create temp dir");
    p
}

/// Minimal hand-crafted PNG bytes with dims 4×3.
fn fake_png_bytes() -> Vec<u8> {
    let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend(&13u32.to_be_bytes()); // IHDR length
    v.extend(b"IHDR");
    v.extend(&4u32.to_be_bytes()); // width
    v.extend(&3u32.to_be_bytes()); // height
    v.extend(&[8, 6, 0, 0, 0]); // depth + color + compression + filter + interlace
    v.extend(&[0; 4]); // pretend CRC
    v
}

#[test]
fn rag_ingest_then_retrieve_returns_topical_passage() {
    build_axon();
    let dir = temp_dir("rag_basic");
    let src = dir.join("p.ax");
    let prog = r#"
fn main() uses { Console } {
    rag_index_new(512)
    rag_ingest("doc1", "Ferret are small carnivorous mammal related to weasel and mink.")
    rag_ingest("doc2", "The stock market closed lower today with tech leading declines.")
    rag_ingest("doc3", "Domestic ferret can be trained to use a litter box and respond to names.")
    let hits = rag_retrieve("how to train a ferret", 1)
    print(list_get(hits, 0).text)
}
"#;
    std::fs::write(&src, prog).unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "run: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Ferret") || stdout.contains("ferret"),
        "expected a ferret passage; got: {stdout:?}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rag_save_and_load_survive_process_restart() {
    build_axon();
    let dir = temp_dir("rag_persist");
    let idx_path = dir.join("idx.json").display().to_string();

    // First run: ingest + save.
    let src1 = dir.join("save.ax");
    std::fs::write(
        &src1,
        format!(
            r#"
fn main() uses {{ Console }} {{
    rag_index_new(512)
    rag_ingest("kb", "ferret pet animal companion litter box")
    rag_ingest("kb", "treasuries yield basis points auction")
    rag_save("{idx_path}")
    print_int(rag_index_len())
}}
"#
        ),
    )
    .unwrap();
    let out1 = Command::new(axon_bin())
        .args(["run", src1.to_str().unwrap()])
        .output()
        .expect("axon run save");
    assert!(out1.status.success(), "save: {:?}", out1);
    assert!(std::path::Path::new(&idx_path).exists(), "index file missing");

    // Second run: load + query.
    let src2 = dir.join("load.ax");
    std::fs::write(
        &src2,
        format!(
            r#"
fn main() uses {{ Console }} {{
    rag_load("{idx_path}")
    let hits = rag_retrieve("treasuries auction", 1)
    print(list_get(hits, 0).text)
}}
"#
        ),
    )
    .unwrap();
    let out2 = Command::new(axon_bin())
        .args(["run", src2.to_str().unwrap()])
        .output()
        .expect("axon run load");
    assert!(out2.status.success(), "load: {:?}", out2);
    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout.contains("treasuries"),
        "expected the finance passage, got: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn media_image_load_reads_png_dimensions() {
    build_axon();
    let dir = temp_dir("media_png");
    let img_path = dir.join("tiny.png");
    std::fs::write(&img_path, fake_png_bytes()).unwrap();

    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        format!(
            r#"
fn main() uses {{ Console }} {{
    let img = media_image_load("{}")
    print_int(img.width)
    print_int(img.height)
    print(img.mime)
}}
"#,
            img_path.display()
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "run: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("4"), "expected width 4 in output: {stdout:?}");
    assert!(stdout.contains("3"), "expected height 3 in output: {stdout:?}");
    assert!(
        stdout.contains("image/png"),
        "expected image/png in output: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn media_sniff_returns_correct_mime() {
    build_axon();
    let dir = temp_dir("media_sniff");
    let png_path = dir.join("a.png");
    std::fs::write(&png_path, fake_png_bytes()).unwrap();

    let src = dir.join("p.ax");
    std::fs::write(
        &src,
        format!(
            r#"
fn main() uses {{ Console }} {{
    let s = media_sniff("{}")
    print(s.kind)
    print(s.mime)
}}
"#,
            png_path.display()
        ),
    )
    .unwrap();
    let out = Command::new(axon_bin())
        .args(["run", src.to_str().unwrap()])
        .output()
        .expect("axon run");
    assert!(out.status.success(), "run: {:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Image"), "got: {stdout:?}");
    assert!(stdout.contains("image/png"), "got: {stdout:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

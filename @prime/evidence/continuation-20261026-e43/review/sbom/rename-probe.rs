use std::fs;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let root = PathBuf::from(r"C:\mDev\Polaris\@prime\tmp\20261026-e43\sbom\rename-probe");
    fs::create_dir_all(&root)?;
    let source = root.join("source.tmp");
    let destination = root.join("destination.txt");
    fs::write(&source, b"new")?;
    fs::write(&destination, b"old")?;
    let rename_result = fs::rename(&source, &destination);
    println!("rename_result={rename_result:?}");
    println!("source_exists={}", source.exists());
    println!("destination={:?}", fs::read(&destination)?);
    if let Err(error) = rename_result {
        return Err(error);
    }
    Ok(())
}

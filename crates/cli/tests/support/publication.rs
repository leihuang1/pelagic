use std::fs;
use std::path::{Path, PathBuf};

pub fn h5(output: &Path, input: &Path) -> PathBuf {
    output.join(format!("{}.h5", output::directory::stem(input).unwrap()))
}

pub fn old_output(output: &Path, bytes: &[u8]) {
    output::directory::publish::<(), String>(
        output,
        &[],
        None,
        output::directory::SyncPolicy::default(),
        |scratch| fs::write(scratch.join("previous.bin"), bytes).map_err(|e| e.to_string()),
    )
    .unwrap();
}

pub fn read_output(output: &Path) -> Vec<u8> {
    if !output.is_dir() {
        return fs::read(output).unwrap();
    }
    let path = fs::read_dir(output)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "h5"))
        .unwrap_or_else(|| output.join("previous.bin"));
    fs::read(path).unwrap()
}

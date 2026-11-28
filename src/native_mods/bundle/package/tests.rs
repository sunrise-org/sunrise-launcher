use super::*;

/// SunnyOS (sunrise#5648) builds RVVM's C sources from its build script: the launcher stages
/// every file the mod tool packs, its `.c` and `.h` files among them. The list is the packed
/// file list of its `aria-neon-dev-snap` package.
#[test]
fn staging_keeps_every_packed_file_of_a_c_source_mod() {
    let packed: Vec<&str> = include_str!("../sunny_os_packed_files.txt")
        .lines()
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("sunny_os");
    for name in &packed {
        let path = project.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = match *name {
            manifest::MANIFEST => "id = 'sunny_os'\nname = 'SunnyOS'\nversion = '0.1.0'\n[native]\ngame = '26.3'\nsunrise = 'a123456789012345678901234567890123456789'\n".into(),
            "Cargo.toml" => "[package]\nname = 'sunny_os'\nversion = '0.1.0'\nedition = '2024'\n".into(),
            USES_FILE => "{}".into(),
            _ => format!("// {name}\n"),
        };
        fs::write(path, bytes).unwrap();
    }
    let bundle = read(&project).unwrap().expect("a native package");
    let staged = dir.path().join("staged");
    copy(&bundle, &staged).unwrap();
    let mut found = Vec::new();
    let mut pending = vec![staged.clone()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let name = path.strip_prefix(&staged).unwrap().to_str().unwrap();
                found.push(name.replace('\\', "/"));
            }
        }
    }
    found.sort();
    let mut expected: Vec<String> = packed.iter().map(|name| name.to_string()).collect();
    expected.sort();
    assert_eq!(found, expected);
    let c_files =
        |names: &[String], suffix: &str| names.iter().filter(|n| n.ends_with(suffix)).count();
    assert_eq!((c_files(&found, ".c"), c_files(&found, ".h")), (61, 79));
}

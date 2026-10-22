//! Multi-file programs: loading the modules named by `use`.
//!
//! `use lib.math` loads `<root>/lib/math.nex`, where `<root>` is the
//! directory of the main file, and makes its functions callable as
//! `math.name(...)`. Every module is loaded once, however many files import
//! it. Modules contain only declarations, so circular imports are allowed.

use crate::ast::{Item, Program};
use crate::diag::Diagnostic;
use crate::parser::parse_file;
use crate::span::SourceFile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One parsed file of a project.
#[derive(Clone, Debug)]
pub struct Unit {
    /// Module name such as `lib.math`; empty for the main file.
    pub name: String,
    /// Index into [`Project::files`].
    pub file: u32,
    pub program: Program,
    /// Local alias (last path segment) to module name.
    pub imports: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct Project {
    /// `units[0]` is the main file.
    pub units: Vec<Unit>,
    pub files: Vec<SourceFile>,
}

/// Reads module sources. Implemented by the CLI for the real file system
/// and by tests for in-memory projects.
pub trait Loader {
    /// The text of the file at `path`, or `None` if it does not exist.
    fn read(&mut self, path: &Path) -> Option<String>;
}

/// Reads modules from disk.
pub struct FsLoader;

impl Loader for FsLoader {
    fn read(&mut self, path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
    }
}

/// A loader with no files: every `use` is unresolved.
pub struct NoLoader;

impl Loader for NoLoader {
    fn read(&mut self, _: &Path) -> Option<String> {
        None
    }
}

/// In-memory files keyed by path relative to the root, using `/`.
pub struct MemLoader(pub HashMap<String, String>);

impl Loader for MemLoader {
    fn read(&mut self, path: &Path) -> Option<String> {
        let key = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        self.0.get(&key).cloned()
    }
}

/// Load `main` and, transitively, every module it uses.
pub fn load(
    main: SourceFile,
    root: &Path,
    loader: &mut dyn Loader,
) -> Result<Project, (Vec<Diagnostic>, Vec<SourceFile>)> {
    let mut files = vec![main];
    let (program, mut next_id) = match parse_file(&files[0].text, 0, 0) {
        Ok(parsed) => parsed,
        Err(d) => return Err((vec![d], files)),
    };
    let mut units = vec![Unit {
        name: String::new(),
        file: 0,
        program,
        imports: HashMap::new(),
    }];
    let mut loaded: HashMap<String, Option<usize>> = HashMap::new();
    let mut diags = Vec::new();

    let mut i = 0;
    while i < units.len() {
        let uses: Vec<_> = units[i]
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Use(u) => Some(u.clone()),
                _ => None,
            })
            .collect();
        for u in uses {
            let segments: Vec<&str> = u.path.iter().map(|p| p.name.as_str()).collect();
            let name = segments.join(".");
            let alias = segments.last().expect("non-empty path").to_string();
            if let Some(previous) = units[i].imports.get(&alias) {
                diags.push(
                    Diagnostic::error(format!("two imports are both called `{alias}`"), u.span)
                        .note(format!("`{previous}` was already imported as `{alias}`")),
                );
                continue;
            }
            if !loaded.contains_key(&name) {
                let rel: PathBuf = segments.iter().collect::<PathBuf>().with_extension("nex");
                match loader.read(&root.join(&rel)) {
                    None => {
                        loaded.insert(name.clone(), None);
                        diags.push(
                            Diagnostic::error(format!("unresolved module `{name}`"), u.span).note(
                                format!(
                                    "looked for `{}` next to the main file",
                                    rel.display().to_string().replace('\\', "/")
                                ),
                            ),
                        );
                    }
                    Some(text) => {
                        let file = files.len() as u32;
                        files.push(SourceFile::new(rel.display().to_string(), text));
                        match parse_file(&files[file as usize].text, file, next_id) {
                            Err(d) => {
                                loaded.insert(name.clone(), None);
                                diags.push(d);
                            }
                            Ok((program, next)) => {
                                next_id = next;
                                for item in &program.items {
                                    if let Item::App(a) = item {
                                        diags.push(
                                            Diagnostic::error(
                                                "a module cannot contain an `app` block",
                                                a.name.span,
                                            )
                                            .note("only the main file may declare apps"),
                                        );
                                    }
                                }
                                loaded.insert(name.clone(), Some(units.len()));
                                units.push(Unit {
                                    name: name.clone(),
                                    file,
                                    program,
                                    imports: HashMap::new(),
                                });
                            }
                        }
                    }
                }
            }
            if loaded.get(&name).is_some_and(Option::is_some) {
                units[i].imports.insert(alias, name);
            }
        }
        i += 1;
    }

    if diags.is_empty() {
        Ok(Project { units, files })
    } else {
        Err((diags, files))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(files: &[(&str, &str)]) -> MemLoader {
        MemLoader(
            files
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn loads_modules_transitively_once() {
        let mut loader = mem(&[
            (
                "lib/math.nex",
                "use util\nfn sq(x: Int) -> Int { return x * x }",
            ),
            ("util.nex", "use lib.math\nfn one() -> Int { return 1 }"),
        ]);
        let main = SourceFile::new("main.nex", "use lib.math\nuse util\napp A { }");
        let p = load(main, Path::new(""), &mut loader).unwrap();
        let names: Vec<&str> = p.units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["", "lib.math", "util"]);
        assert_eq!(p.units[0].imports["math"], "lib.math");
        assert_eq!(p.units[1].imports["util"], "util");
        assert_eq!(p.units[2].imports["math"], "lib.math");
        assert_eq!(p.files.len(), 3);
        assert_eq!(p.files[1].name.replace('\\', "/"), "lib/math.nex");
    }

    #[test]
    fn reports_missing_modules_apps_and_alias_clashes() {
        let mut loader = mem(&[("a/x.nex", "fn f() {}"), ("b/x.nex", "app Nope {}")]);
        let main = SourceFile::new("main.nex", "use net.http\nuse a.x\nuse b.x\napp A { }");
        let (diags, _) = load(main, Path::new(""), &mut loader).unwrap_err();
        let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
        assert!(
            msgs.iter().any(|m| m == "unresolved module `net.http`"),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.contains("both called `x`")),
            "{msgs:?}"
        );

        let mut loader = mem(&[("b/x.nex", "app Nope {}")]);
        let main = SourceFile::new("main.nex", "use b.x\napp A { }");
        let (diags, _) = load(main, Path::new(""), &mut loader).unwrap_err();
        assert!(diags[0].message.contains("cannot contain an `app`"));
        assert_eq!(diags[0].span.file, 1);
    }

    #[test]
    fn syntax_errors_in_modules_point_at_the_module() {
        let mut loader = mem(&[("m.nex", "fn f( {")]);
        let main = SourceFile::new("main.nex", "use m\napp A { }");
        let (diags, files) = load(main, Path::new(""), &mut loader).unwrap_err();
        assert_eq!(diags[0].span.file, 1);
        assert_eq!(files[1].name, "m.nex");
    }
}

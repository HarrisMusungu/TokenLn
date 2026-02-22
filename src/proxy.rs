use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FrontendKind {
    CargoTest,
    CargoBuild,
    GoTest,
    Pytest,
    Jest,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ProxyRoute {
    Analyze(FrontendKind),
    Passthrough,
}

pub fn classify_command(program: &str, args: &[String]) -> ProxyRoute {
    let binary_name = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);

    match binary_name {
        "cargo" => match args.first().map(String::as_str) {
            Some("test") => ProxyRoute::Analyze(FrontendKind::CargoTest),
            Some("build") => ProxyRoute::Analyze(FrontendKind::CargoBuild),
            _ => ProxyRoute::Passthrough,
        },
        "go" => match args.first().map(String::as_str) {
            Some("test") => ProxyRoute::Analyze(FrontendKind::GoTest),
            _ => ProxyRoute::Passthrough,
        },
        "pytest" => ProxyRoute::Analyze(FrontendKind::Pytest),
        "jest" => ProxyRoute::Analyze(FrontendKind::Jest),
        _ => ProxyRoute::Passthrough,
    }
}

pub fn supported_wrapper_commands() -> &'static [&'static str] {
    &["cargo", "go", "pytest", "jest"]
}

pub fn resolve_real_program(command: &str, skip_dir: &Path) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        if same_directory(&dir, skip_dir) {
            continue;
        }

        let candidate = dir.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

pub fn render_wrapper_script(tokenln_bin: &Path, target: &str, real_program: &Path) -> String {
    let tokenln = shell_quote(&tokenln_bin.to_string_lossy());
    let target = shell_quote(target);
    let program = shell_quote(&real_program.to_string_lossy());

    format!(
        "#!/usr/bin/env bash\n\
set -euo pipefail\n\
exec {tokenln} proxy run --target {target} -- {program} \"$@\"\n"
    )
}

fn same_directory(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    match (left.canonicalize(), right.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn shell_quote(input: &str) -> String {
    let escaped = input.replace('\'', "'\\''");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests {
    use super::{classify_command, render_wrapper_script, FrontendKind, ProxyRoute};
    use std::path::Path;

    #[test]
    fn classifies_supported_frontends() {
        assert_eq!(
            classify_command("cargo", &["test".to_string(), "--lib".to_string()]),
            ProxyRoute::Analyze(FrontendKind::CargoTest)
        );
        assert_eq!(
            classify_command("cargo", &["build".to_string()]),
            ProxyRoute::Analyze(FrontendKind::CargoBuild)
        );
        assert_eq!(
            classify_command("go", &["test".to_string(), "./...".to_string()]),
            ProxyRoute::Analyze(FrontendKind::GoTest)
        );
        assert_eq!(
            classify_command("pytest", &["-q".to_string()]),
            ProxyRoute::Analyze(FrontendKind::Pytest)
        );
        assert_eq!(
            classify_command("/usr/local/bin/jest", &["--runInBand".to_string()]),
            ProxyRoute::Analyze(FrontendKind::Jest)
        );
    }

    #[test]
    fn unsupported_subcommands_fall_back_to_passthrough() {
        assert_eq!(
            classify_command("cargo", &["metadata".to_string()]),
            ProxyRoute::Passthrough
        );
        assert_eq!(
            classify_command("go", &["env".to_string()]),
            ProxyRoute::Passthrough
        );
        assert_eq!(
            classify_command("python", &["-m".to_string(), "pytest".to_string()]),
            ProxyRoute::Passthrough
        );
    }

    #[test]
    fn wrapper_script_embeds_target_and_paths() {
        let script = render_wrapper_script(
            Path::new("/tmp/tokenln"),
            "claude",
            Path::new("/usr/local/bin/pytest"),
        );
        assert!(script.contains("'claude'"));
        assert!(script.contains("proxy run --target"));
        assert!(script.contains("'/usr/local/bin/pytest'"));
    }
}

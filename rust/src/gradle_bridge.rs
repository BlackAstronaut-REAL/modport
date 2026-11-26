use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::workspace_validation::{validate_workspace, WorkspaceValidationReport};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompileDiagnostic {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradleCompileReport {
    pub workspace: PathBuf,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub duration_seconds: f64,
    pub log_file: PathBuf,
    pub report_file: PathBuf,
    pub diagnostics: Vec<CompileDiagnostic>,
    pub validation_ok: Option<bool>,
    pub validation_errors: usize,
    pub validation_warnings: usize,
    pub stdout: String,
    pub stderr: String,
}

fn gradle_command(root: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    let candidates = ["gradlew.bat", "gradlew"];
    #[cfg(not(windows))]
    let candidates = ["gradlew"];

    for name in candidates {
        let candidate = root.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    #[cfg(windows)]
    let executable = PathBuf::from("gradle.bat");
    #[cfg(not(windows))]
    let executable = PathBuf::from("gradle");
    Ok(executable)
}

fn windows_command_line(executable: &Path, args: &[String]) -> Result<String> {
    let raw_path = executable.to_string_lossy();
    let executable_text = if let Some(unc) = raw_path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(local) = raw_path.strip_prefix(r"\\?\") {
        local.to_owned()
    } else {
        raw_path.into_owned()
    };
    let mut values = Vec::with_capacity(args.len() + 1);
    values.push(executable_text);
    values.extend(args.iter().cloned());

    // cmd.exe /S /C treats the first and last quote specially. Pass one
    // explicitly quoted command string so wrapper paths with spaces and
    // Gradle arguments containing shell metacharacters stay individual args.
    // Quotes/newlines are rejected rather than being interpreted as cmd syntax.
    let mut quoted = Vec::with_capacity(values.len());
    for value in values {
        if value.contains('"') || value.contains('\r') || value.contains('\n') {
            bail!("Gradle executable/arguments cannot contain quotes or newlines when using the Windows batch wrapper");
        }
        quoted.push(format!("\"{value}\""));
    }
    Ok(quoted.join(" "))
}

fn parse_diagnostics(output: &str) -> Vec<CompileDiagnostic> {
    let mut diagnostics = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        // Keep the compiler's line-level diagnostics and one top-level
        // Gradle failure summary. Task progress lines such as
        // "> Task :compileJava FAILED" are useful context in the full log but
        // duplicate the final BUILD FAILED result and are not diagnostics.
        let severity = if lower.contains(" error:") || lower.starts_with("error:") {
            Some("error")
        } else if lower.contains(" warning:") || lower.starts_with("warning:") {
            Some("warning")
        } else if lower.starts_with("build failed") || lower.contains("execution failed for task") {
            Some("error")
        } else {
            None
        };
        if let Some(severity) = severity {
            diagnostics.push(CompileDiagnostic {
                severity: severity.to_owned(),
                message: trimmed.to_owned(),
            });
        }
    }
    diagnostics
}

fn write_report(report: &GradleCompileReport) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    fs::write(&report.report_file, format!("{json}\n"))
        .with_context(|| format!("Writing Gradle report {}", report.report_file.display()))
}

/// Run the workspace's own Gradle wrapper (preferred) or a system Gradle install.
///
/// This deliberately invokes the existing MCreator-generated build rather than
/// synthesizing a second Gradle project. The build log and structured report are
/// retained even when Gradle exits unsuccessfully.
pub fn compile_workspace(
    workspace_root: &Path,
    tasks: &[String],
    offline: bool,
    skip_validation: bool,
    fail_on_validation: bool,
) -> Result<GradleCompileReport> {
    let root = fs::canonicalize(workspace_root)
        .with_context(|| format!("Resolving workspace {}", workspace_root.display()))?;
    if !root.is_dir() {
        bail!("Workspace path is not a directory: {}", root.display());
    }
    if !root.join("build.gradle").is_file() && !root.join("build.gradle.kts").is_file() {
        bail!("No build.gradle or build.gradle.kts exists in {}", root.display());
    }

    let validation: Option<WorkspaceValidationReport> = if skip_validation {
        None
    } else {
        Some(validate_workspace(&root, None, true)?)
    };
    let validation_ok = validation.as_ref().map(|value| value.ok);
    let validation_errors = validation.as_ref().map(|value| value.errors.len()).unwrap_or(0);
    let validation_warnings = validation.as_ref().map(|value| value.warnings.len()).unwrap_or(0);

    let executable = gradle_command(&root)?;
    let selected_tasks = if tasks.is_empty() {
        vec!["build".to_owned()]
    } else {
        tasks.to_vec()
    };
    let mut args = vec![
        "--no-daemon".to_owned(),
        "--console=plain".to_owned(),
        "--stacktrace".to_owned(),
    ];
    if offline {
        args.push("--offline".to_owned());
    }
    args.extend(selected_tasks);
    #[cfg(windows)]
    let mut command = {
        // A batch wrapper must run under cmd.exe. raw_arg is deliberate:
        // cmd.exe does not use the normal Windows argv backslash-quote rules.
        // Pass /C plus its fully quoted command string verbatim.
        use std::os::windows::process::CommandExt;
        let command_line = windows_command_line(&executable, &args)?;
        let mut command = Command::new("cmd.exe");
        command.raw_arg(format!(" /D /S /C \"{command_line}\""));
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new(&executable);
        command.args(&args);
        command
    };
    command.current_dir(&root);

    let started = Instant::now();
    let result = command.output()
        .with_context(|| format!("Starting Gradle command '{}' in {}", executable.display(), root.display()));
    let duration_seconds = started.elapsed().as_secs_f64();

    let (exit_code, success, stdout, stderr) = match result {
        Ok(output) => (
            output.status.code(),
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => (None, false, String::new(), error.to_string()),
    };
    let combined = if stderr.is_empty() { stdout.clone() } else { format!("{stdout}\n{stderr}") };
    let diagnostics = parse_diagnostics(&combined);
    let log_file = root.join("MOJMODELBRIDGE_GRADLE_BUILD.log");
    let report_file = root.join("MOJMODELBRIDGE_GRADLE_BUILD.json");
    fs::write(&log_file, &combined)
        .with_context(|| format!("Writing Gradle log {}", log_file.display()))?;

    let mut full_command = vec![executable.display().to_string()];
    full_command.extend(args);
    let report = GradleCompileReport {
        workspace: root,
        command: full_command,
        exit_code,
        success: success && !(fail_on_validation && validation_ok == Some(false)),
        duration_seconds,
        log_file,
        report_file,
        diagnostics,
        validation_ok,
        validation_errors,
        validation_warnings,
        stdout,
        stderr,
    };
    write_report(&report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invokes_workspace_wrapper_and_persists_compile_report() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("MCreator Workspace");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("build.gradle"), "plugins { }\n").unwrap();
        fs::write(
            root.join("test.mcreator"),
            serde_json::to_vec(&serde_json::json!({
                "mcreatorVersion": 202600,
                "workspaceSettings": {
                    "modid": "converted",
                    "modElementsPackage": "net.converted",
                    "currentGenerator": "neoforge-1.21.1"
                },
                "mod_elements": [],
                "language_map": {"en_us": {}}
            })).unwrap(),
        ).unwrap();

        #[cfg(windows)]
        fs::write(
            root.join("gradlew.bat"),
            "@echo off\r\necho Gradle wrapper test output\r\nexit /b 0\r\n",
        ).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let wrapper = root.join("gradlew");
            fs::write(&wrapper, "#!/bin/sh\necho 'Gradle wrapper test output'\nexit 0\n").unwrap();
            let mut permissions = fs::metadata(&wrapper).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&wrapper, permissions).unwrap();
        }

        let report = compile_workspace(
            &root,
            &["testTask".to_owned()],
            false,
            false,
            false,
        ).unwrap();
        assert!(report.success, "wrapper failed: {}", report.stderr);
        assert_eq!(report.exit_code, Some(0));
        assert!(report.validation_ok.unwrap());
        assert!(report.log_file.is_file());
        assert!(report.report_file.is_file());
        assert!(fs::read_to_string(&report.log_file).unwrap().contains("Gradle wrapper test output"));
        assert!(report.command.first().unwrap().to_ascii_lowercase().contains("gradlew"));
    }

    #[test]
    fn strips_windows_extended_path_prefix_for_cmd() {
        let executable = Path::new(r"\\?\C:\Users\Dev User\MCreator Workspace\gradlew.bat");
        let args = vec!["build".to_owned()];
        assert_eq!(
            windows_command_line(executable, &args).unwrap(),
            r#""C:\Users\Dev User\MCreator Workspace\gradlew.bat" "build""#,
        );
    }

    #[test]
    fn quotes_windows_wrapper_paths_and_gradle_arguments_for_cmd() {
        let executable = Path::new(r"C:\Users\Dev User\MCreator Workspace\gradlew.bat");
        let args = vec!["--no-daemon".to_owned(), "compileJava".to_owned()];
        assert_eq!(
            windows_command_line(executable, &args).unwrap(),
            r#""C:\Users\Dev User\MCreator Workspace\gradlew.bat" "--no-daemon" "compileJava""#,
        );
    }

    #[test]
    fn extracts_compiler_errors_and_warnings_without_gradle_noise() {
        let output = "> Task :compileJava FAILED\nFoo.java:12: error: cannot find symbol\n  symbol: method missing()\nNote: recompile with -Xlint:deprecation\nBar.java:8: warning: unchecked conversion\nBUILD FAILED";
        let diagnostics = parse_diagnostics(output);
        assert_eq!(diagnostics.len(), 3);
        assert_eq!(diagnostics[0].severity, "error");
        assert_eq!(diagnostics[1].severity, "warning");
        assert_eq!(diagnostics[2].severity, "error");
    }
}

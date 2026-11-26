//! Native NeoForge 1.21.1 project scaffold generation.
//!
//! This establishes a real Gradle/Java output target. Content adapters can
//! populate the generated registration modules and resource tree in later passes.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

pub const TARGET_MINECRAFT_VERSION: &str = "1.21.1";
pub const DEFAULT_NEOFORGE_VERSION: &str = "21.1.235";
pub const DEFAULT_GRADLE_VERSION: &str = "9.2.1";

#[derive(Debug, Clone, Serialize)]
pub struct NeoForgeProjectReport {
    pub output_root: PathBuf,
    pub mod_id: String,
    pub mod_name: String,
    pub package: String,
    pub minecraft_version: String,
    pub neoforge_version: String,
    pub generated_files: Vec<String>,
    pub status: String,
    pub warnings: Vec<String>,
}

pub fn create_neoforge_project(
    output_root: &Path,
    mod_id: &str,
    mod_name: &str,
    package: &str,
    neoforge_version: &str,
    overwrite: bool,
) -> Result<NeoForgeProjectReport> {
    validate_mod_id(mod_id)?;
    validate_mod_name(mod_name)?;
    validate_package(package)?;
    validate_version(neoforge_version)?;

    let package_path = package.replace('.', "/");
    let java_root = format!("src/main/java/{package_path}");
    let mut files: Vec<(String, String)> = Vec::new();

    files.push(("settings.gradle".to_owned(), settings_gradle().to_owned()));
    files.push(("build.gradle".to_owned(), build_gradle().to_owned()));
    files.push(("gradlew.bat".to_owned(), gradlew_bat().to_owned()));
    files.push(("gradlew".to_owned(), gradlew_unix().to_owned()));
    files.push(("gradle/wrapper/gradle-wrapper.properties".to_owned(), gradle_wrapper_properties().to_owned()));
    files.push(("gradle.properties".to_owned(), render(
        gradle_properties(),
        &[
            ("@MOD_ID@", mod_id),
            ("@MOD_NAME@", &escape_property(mod_name)),
            ("@PACKAGE@", package),
            ("@NEO_VERSION@", neoforge_version),
        ],
    )));
    files.push(("src/main/resources/META-INF/neoforge.mods.toml".to_owned(), render(
        mod_toml(),
        &[
            ("@MOD_ID@", mod_id),
            ("@MOD_NAME@", &escape_toml(mod_name)),
            ("@NEO_VERSION@", neoforge_version),
        ],
    )));
    files.push(("src/main/resources/pack.mcmeta".to_owned(), format!(
        "{{\n  \"pack\": {{\n    \"description\": {},\n    \"pack_format\": 34\n  }}\n}}\n",
        serde_json::to_string(&format!("{mod_name} resources"))?
    )));
    files.push((format!("{java_root}/NeoForgeMod.java"), render(
        mod_entrypoint(),
        &[("@PACKAGE@", package), ("@MOD_ID@", mod_id)],
    )));
    files.push((format!("{java_root}/registry/ModBlocks.java"), render(
        blocks_registry(),
        &[("@PACKAGE@", package), ("@MOD_ID@", mod_id)],
    )));
    files.push((format!("{java_root}/registry/ModItems.java"), render(
        items_registry(),
        &[("@PACKAGE@", package), ("@MOD_ID@", mod_id)],
    )));
    files.push((format!("{java_root}/registry/ModEntities.java"), render(
        entities_registry(),
        &[("@PACKAGE@", package), ("@MOD_ID@", mod_id)],
    )));
    files.push((format!("{java_root}/registry/ModBlockEntities.java"), render(
        block_entities_registry(),
        &[("@PACKAGE@", package), ("@MOD_ID@", mod_id)],
    )));
    files.push((".gitignore".to_owned(), gitignore().to_owned()));
    files.push(("README.md".to_owned(), render(
        project_readme(),
        &[
            ("@MOD_NAME@", mod_name),
            ("@MOD_ID@", mod_id),
            ("@PACKAGE@", package),
            ("@NEO_VERSION@", neoforge_version),
        ],
    )));
    files.push(("modport/port-status.json".to_owned(), serde_json::to_string_pretty(&json!({
        "format_version": 1,
        "target": {
            "loader": "NeoForge",
            "minecraft_version": TARGET_MINECRAFT_VERSION,
            "neoforge_version": neoforge_version,
            "java": 21
        },
        "mod": {
            "mod_id": mod_id,
            "display_name": mod_name,
            "package": package
        },
        "conversion_state": "scaffold_only",
        "behavior_fully_reconstructed": false,
        "pending_adapters": [
            "items_and_tools", "armor", "blocks_and_block_states",
            "block_entities", "plants_and_crops", "entities_and_ai",
            "renderers_models_and_animations", "recipes_and_loot",
            "world_generation", "networking", "capabilities_and_data_storage",
            "events_and_procedures", "client_resources_and_gui"
        ],
        "notes": [
            "This is a compilable project foundation, not a claim that source mod behavior has already been ported.",
            "Generated registration classes are intentionally empty until source-driven adapters populate them."
        ]
    }))? + "\n"));

    // Check every collision before writing, so refusal never leaves a partial project.
    let collisions: Vec<String> = files.iter()
        .map(|(relative, _)| output_root.join(relative))
        .filter(|path| path.exists())
        .map(|path| path.strip_prefix(output_root).unwrap_or(&path).to_string_lossy().to_string())
        .collect();
    if !overwrite && !collisions.is_empty() {
        bail!(
            "NeoForge project output contains generated-file collisions (use --overwrite to replace only these files): {}",
            collisions.join(", ")
        );
    }

    fs::create_dir_all(output_root)
        .with_context(|| format!("creating NeoForge project directory {}", output_root.display()))?;
    let mut generated_files = Vec::with_capacity(files.len());
    for (relative, content) in files {
        let destination = output_root.join(&relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating project directory {}", parent.display()))?;
        }
        let content = if relative == "gradlew.bat" {
            content.replace('\n', "\r\n")
        } else {
            content
        };
        fs::write(&destination, content)
            .with_context(|| format!("writing generated NeoForge project file {}", destination.display()))?;
        if relative == "gradlew" {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut permissions = fs::metadata(&destination)?.permissions();
                permissions.set_mode(0o755);
                fs::set_permissions(&destination, permissions)?;
            }
        }
        generated_files.push(relative);
    }
    generated_files.sort();

    Ok(NeoForgeProjectReport {
        output_root: output_root.to_path_buf(),
        mod_id: mod_id.to_owned(),
        mod_name: mod_name.to_owned(),
        package: package.to_owned(),
        minecraft_version: TARGET_MINECRAFT_VERSION.to_owned(),
        neoforge_version: neoforge_version.to_owned(),
        generated_files,
        status: "scaffold_only".to_owned(),
        warnings: vec![
            "No legacy source behavior has been converted by this scaffold generator.".to_owned(),
            "Run the source-driven content adapters before treating the generated project as a completed port.".to_owned(),
            "The generated gradlew / gradlew.bat launcher downloads the pinned Gradle distribution on first build; internet access is required for that initial download.".to_owned(),
        ],
    })
}

fn validate_mod_id(value: &str) -> Result<()> {
    let valid = (2..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
    if !valid {
        bail!("mod ID must match [a-z][a-z0-9_]{{1,63}}; received '{value}'");
    }
    Ok(())
}

fn validate_mod_name(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(|character| character.is_control()) {
        bail!("mod display name must be non-empty and contain no control characters");
    }
    Ok(())
}

fn validate_package(value: &str) -> Result<()> {
    let keywords = [
        "abstract", "assert", "boolean", "break", "byte", "case", "catch", "char",
        "class", "const", "continue", "default", "do", "double", "else", "enum",
        "extends", "final", "finally", "float", "for", "goto", "if", "implements",
        "import", "instanceof", "int", "interface", "long", "native", "new",
        "package", "private", "protected", "public", "return", "short", "static",
        "strictfp", "super", "switch", "synchronized", "this", "throw", "throws",
        "transient", "try", "void", "volatile", "while", "true", "false", "null",
    ];
    let segments: Vec<&str> = value.split('.').collect();
    if segments.is_empty() || segments.iter().any(|segment| {
        segment.is_empty()
            || keywords.contains(segment)
            || !segment.chars().enumerate().all(|(index, character)| {
                if index == 0 {
                    character.is_ascii_alphabetic() || character == '_' || character == '$'
                } else {
                    character.is_ascii_alphanumeric() || character == '_' || character == '$'
                }
            })
    }) {
        bail!("package must be a dot-separated sequence of valid Java identifiers; received '{value}'");
    }
    Ok(())
}

fn validate_version(value: &str) -> Result<()> {
    if value.is_empty() || !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+' | '_')
    }) {
        bail!("NeoForge version contains unsupported characters: '{value}'");
    }
    Ok(())
}

fn escape_property(value: &str) -> String {
    value.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('=', "\\=")
        .replace('#', "\\#")
        .replace('!', "\\!")
}

fn escape_toml(value: &str) -> String {
    value.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn render(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut result = template.to_owned();
    for (needle, value) in replacements {
        result = result.replace(needle, value);
    }
    result
}

fn gradlew_bat() -> &'static str {
    r#"@echo off
setlocal
set "MODPORT_GRADLE_VERSION=9.2.1"
if defined GRADLE_USER_HOME (
    set "MODPORT_GRADLE_USER_HOME=%GRADLE_USER_HOME%"
) else (
    set "MODPORT_GRADLE_USER_HOME=%USERPROFILE%\.gradle"
)
set "MODPORT_GRADLE_CACHE=%MODPORT_GRADLE_USER_HOME%\wrapper\dists\modport-gradle-%MODPORT_GRADLE_VERSION%"
set "MODPORT_GRADLE_HOME=%MODPORT_GRADLE_CACHE%\gradle-%MODPORT_GRADLE_VERSION%"
set "MODPORT_GRADLE_CMD=%MODPORT_GRADLE_HOME%\bin\gradle.bat"

if not exist "%MODPORT_GRADLE_CMD%" (
    echo Modport: downloading Gradle %MODPORT_GRADLE_VERSION% on first build...
    powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference = 'Stop'; $gradleUserHome = if ($env:GRADLE_USER_HOME) { $env:GRADLE_USER_HOME } else { Join-Path $env:USERPROFILE '.gradle' }; $root = Join-Path $gradleUserHome 'wrapper\dists\modport-gradle-9.2.1'; New-Item -ItemType Directory -Force -Path $root | Out-Null; $zip = Join-Path $root 'gradle-9.2.1-bin.zip'; Invoke-WebRequest -UseBasicParsing -Uri 'https://services.gradle.org/distributions/gradle-9.2.1-bin.zip' -OutFile $zip; Add-Type -AssemblyName System.IO.Compression.FileSystem; [System.IO.Compression.ZipFile]::ExtractToDirectory($zip, $root); Remove-Item -LiteralPath $zip -Force"
    if errorlevel 1 (
        echo Modport: Gradle download failed. Check your internet connection and retry.
        exit /b 1
    )
)

if not exist "%MODPORT_GRADLE_CMD%" (
    echo Modport: Gradle was not found after downloading the distribution.
    exit /b 1
)

call "%MODPORT_GRADLE_CMD%" %*
exit /b %ERRORLEVEL%
"#
}

fn gradlew_unix() -> &'static str {
    r#"#!/usr/bin/env sh
set -eu

GRADLE_VERSION="9.2.1"
GRADLE_USER_HOME="${GRADLE_USER_HOME:-"$HOME/.gradle"}"
GRADLE_CACHE="${GRADLE_USER_HOME}/wrapper/dists/modport-gradle-${GRADLE_VERSION}"
GRADLE_HOME="${GRADLE_CACHE}/gradle-${GRADLE_VERSION}"
GRADLE_COMMAND="${GRADLE_HOME}/bin/gradle"

if [ ! -x "${GRADLE_COMMAND}" ]; then
    mkdir -p "${GRADLE_CACHE}"
    ARCHIVE="${GRADLE_CACHE}/gradle-${GRADLE_VERSION}-bin.zip"
    URL="https://services.gradle.org/distributions/gradle-${GRADLE_VERSION}-bin.zip"
    echo "Modport: downloading Gradle ${GRADLE_VERSION} on first build..."
    if command -v curl >/dev/null 2>&1; then
        curl -fL "${URL}" -o "${ARCHIVE}"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "${ARCHIVE}" "${URL}"
    else
        echo "Modport: curl or wget is required to download Gradle." >&2
        exit 1
    fi
    if ! command -v unzip >/dev/null 2>&1; then
        echo "Modport: unzip is required to install the Gradle distribution." >&2
        exit 1
    fi
    unzip -q -o "${ARCHIVE}" -d "${GRADLE_CACHE}"
    rm -f "${ARCHIVE}"
fi

exec "${GRADLE_COMMAND}" "$@"
"#
}

fn gradle_wrapper_properties() -> &'static str {
    r#"distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-9.2.1-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
"#
}

/// Ensure an existing Gradle project has a self-bootstrapping launcher for
/// the current platform. This also upgrades projects generated by older
/// Modport versions when they are opened in Build & Test.
pub(crate) fn ensure_gradle_launcher(output_root: &Path) -> Result<()> {
    #[cfg(windows)]
    let launcher = output_root.join("gradlew.bat");
    #[cfg(not(windows))]
    let launcher = output_root.join("gradlew");

    if launcher.is_file() {
        return Ok(());
    }

    fs::create_dir_all(output_root)
        .with_context(|| format!("creating Gradle project directory {}", output_root.display()))?;
    #[cfg(windows)]
    fs::write(&launcher, gradlew_bat().replace('\n', "\r\n"))
        .with_context(|| format!("writing Gradle launcher {}", launcher.display()))?;
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&launcher, gradlew_unix())
            .with_context(|| format!("writing Gradle launcher {}", launcher.display()))?;
        let mut permissions = fs::metadata(&launcher)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&launcher, permissions)?;
    }

    let properties = output_root.join("gradle/wrapper/gradle-wrapper.properties");
    if !properties.is_file() {
        if let Some(parent) = properties.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&properties, gradle_wrapper_properties())
            .with_context(|| format!("writing Gradle wrapper properties {}", properties.display()))?;
    }
    Ok(())
}

fn settings_gradle() -> &'static str {
    r#"pluginManagement {
    repositories {
        gradlePluginPortal()
        maven { url = 'https://maven.neoforged.net/releases' }
    }
}

plugins {
    id 'org.gradle.toolchains.foojay-resolver-convention' version '1.0.0'
}
"#
}

fn build_gradle() -> &'static str {
    r#"plugins {
    id 'java-library'
    id 'maven-publish'
    id 'net.neoforged.gradle.userdev' version '7.1.38'
}

version = mod_version
group = mod_group_id

sourceSets.main.resources {
    srcDir('src/generated/resources')
}

repositories {
}

base {
    archivesName = mod_id
}

java.toolchain.languageVersion = JavaLanguageVersion.of(21)

runs {
    configureEach {
        systemProperty 'forge.logging.markers', 'REGISTRIES'
        systemProperty 'forge.logging.console.level', 'debug'
        workingDirectory project.layout.projectDirectory.dir('run').dir(name)
        modSource project.sourceSets.main
    }

    client {
        systemProperty 'neoforge.enabledGameTestNamespaces', project.mod_id
    }

    server {
        systemProperty 'neoforge.enabledGameTestNamespaces', project.mod_id
        argument '--nogui'
    }

    gameTestServer {
        systemProperty 'neoforge.enabledGameTestNamespaces', project.mod_id
    }

    data {
        arguments.addAll '--mod', project.mod_id, '--all',
            '--output', file('src/generated/resources/').getAbsolutePath(),
            '--existing', file('src/main/resources/').getAbsolutePath()
    }
}

configurations {
    runtimeClasspath.extendsFrom localRuntime
}

dependencies {
    implementation "net.neoforged:neoforge:$neo_version"
}

tasks.withType(JavaCompile).configureEach {
    options.encoding = 'UTF-8'
}
"#
}

fn gradle_properties() -> &'static str {
    r#"org.gradle.jvmargs=-Xmx2G
org.gradle.daemon=true
org.gradle.parallel=true
org.gradle.caching=true
org.gradle.configuration-cache=true

neogradle.subsystems.parchment.minecraftVersion=1.21.1
neogradle.subsystems.parchment.mappingsVersion=2024.11.17

minecraft_version=1.21.1
minecraft_version_range=[1.21.1]
neo_version=@NEO_VERSION@
loader_version_range=[1,)

mod_id=@MOD_ID@
mod_name=@MOD_NAME@
mod_license=MIT
mod_version=1.0.0
mod_group_id=@PACKAGE@
"#
}

fn mod_toml() -> &'static str {
    r#"modLoader="javafml"
loaderVersion="[1,)"
license="MIT"

[[mods]]
modId="@MOD_ID@"
version="1.0.0"
displayName="@MOD_NAME@"
description='''
Generated by Modport as a NeoForge 1.21.1 target project.
'''

[[dependencies.@MOD_ID@]]
modId="neoforge"
type="required"
versionRange="[@NEO_VERSION@,)"
ordering="NONE"
side="BOTH"

[[dependencies.@MOD_ID@]]
modId="minecraft"
type="required"
versionRange="[1.21.1]"
ordering="NONE"
side="BOTH"
"#
}

fn mod_entrypoint() -> &'static str {
    r#"package @PACKAGE@;

import @PACKAGE@.registry.ModBlockEntities;
import @PACKAGE@.registry.ModBlocks;
import @PACKAGE@.registry.ModEntities;
import @PACKAGE@.registry.ModItems;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;

@Mod("@MOD_ID@")
public final class NeoForgeMod {
    public static final String MOD_ID = "@MOD_ID@";

    public NeoForgeMod(IEventBus modEventBus) {
        ModBlocks.register(modEventBus);
        ModItems.register(modEventBus);
        ModEntities.register(modEventBus);
        ModBlockEntities.register(modEventBus);
    }
}
"#
}

fn blocks_registry() -> &'static str {
    r#"package @PACKAGE@.registry;

import @PACKAGE@.NeoForgeMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.block.Block;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModBlocks {
    public static final DeferredRegister<Block> BLOCKS =
        DeferredRegister.create(Registries.BLOCK, NeoForgeMod.MOD_ID);

    private ModBlocks() {}

    public static void register(IEventBus modEventBus) {
        BLOCKS.register(modEventBus);
    }
}
"#
}

fn items_registry() -> &'static str {
    r#"package @PACKAGE@.registry;

import @PACKAGE@.NeoForgeMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.Item;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModItems {
    public static final DeferredRegister<Item> ITEMS =
        DeferredRegister.create(Registries.ITEM, NeoForgeMod.MOD_ID);

    private ModItems() {}

    public static void register(IEventBus modEventBus) {
        ITEMS.register(modEventBus);
    }
}
"#
}

fn entities_registry() -> &'static str {
    r#"package @PACKAGE@.registry;

import @PACKAGE@.NeoForgeMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.entity.EntityType;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModEntities {
    public static final DeferredRegister<EntityType<?>> ENTITY_TYPES =
        DeferredRegister.create(Registries.ENTITY_TYPE, NeoForgeMod.MOD_ID);

    private ModEntities() {}

    public static void register(IEventBus modEventBus) {
        ENTITY_TYPES.register(modEventBus);
    }
}
"#
}

fn block_entities_registry() -> &'static str {
    r#"package @PACKAGE@.registry;

import @PACKAGE@.NeoForgeMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.registries.DeferredRegister;

public final class ModBlockEntities {
    public static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITY_TYPES =
        DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, NeoForgeMod.MOD_ID);

    private ModBlockEntities() {}

    public static void register(IEventBus modEventBus) {
        BLOCK_ENTITY_TYPES.register(modEventBus);
    }
}
"#
}

fn gitignore() -> &'static str {
    r#".gradle/
build/
run/
.idea/
*.iml
out/
"#
}

fn project_readme() -> &'static str {
    r#"# @MOD_NAME@

NeoForge 1.21.1 project generated by Modport.

- Mod ID: @MOD_ID@
- Package: @PACKAGE@
- Minecraft: 1.21.1
- NeoForge: @NEO_VERSION@
- Java: 21

## Build

Run the included self-bootstrapping Gradle launcher:

    .\gradlew.bat build

The first build downloads Gradle 9.2.1 from the official Gradle distribution
service into your user Gradle cache. No separate Gradle installation is needed.
On Linux/macOS, run:

    ./gradlew build

Java 21 is still required for NeoForge. The Gradle distribution is cached and
reused by later builds.

## Port status

See modport/port-status.json. This is currently a project scaffold. No legacy
mod behavior has been claimed as converted yet. Modport's source-driven adapters
will populate the registry modules, resources, models, events, and behavior.
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_a_neoforge_1_21_1_gradle_project() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("converted");
        let report = create_neoforge_project(
            &output, "example_mod", "Example Mod", "com.example.examplemod",
            DEFAULT_NEOFORGE_VERSION, false
        ).unwrap();

        assert_eq!(report.status, "scaffold_only");
        assert_eq!(report.minecraft_version, "1.21.1");
        assert!(output.join("build.gradle").is_file());
        assert!(output.join("gradlew.bat").is_file());
        assert!(output.join("gradlew").is_file());
        assert!(output.join("gradle/wrapper/gradle-wrapper.properties").is_file());
        assert!(fs::read_to_string(output.join("gradlew.bat")).unwrap().contains("services.gradle.org/distributions/gradle-9.2.1-bin.zip"));
        assert!(output.join("src/main/resources/META-INF/neoforge.mods.toml").is_file());
        assert!(output.join("src/main/java/com/example/examplemod/NeoForgeMod.java").is_file());
        assert!(output.join("modport/port-status.json").is_file());

        let build = fs::read_to_string(output.join("build.gradle")).unwrap();
        assert!(build.contains("net.neoforged.gradle.userdev"));
        assert!(build.contains("net.neoforged:neoforge:"));
        let entrypoint = fs::read_to_string(output.join("src/main/java/com/example/examplemod/NeoForgeMod.java")).unwrap();
        assert!(entrypoint.contains("@Mod(\"example_mod\")"));
        assert!(entrypoint.contains("ModEntities.register(modEventBus)"));
    }

    #[test]
    fn refuses_to_overwrite_existing_project_files_without_permission() {
        let temp = tempdir().unwrap();
        let output = temp.path();
        fs::write(output.join("settings.gradle"), "user content").unwrap();
        let error = create_neoforge_project(
            output, "example_mod", "Example Mod", "com.example", DEFAULT_NEOFORGE_VERSION, false
        ).unwrap_err();
        assert!(format!("{error:#}").contains("collisions"));
        assert_eq!(fs::read_to_string(output.join("settings.gradle")).unwrap(), "user content");
    }

    #[test]
    fn validates_mod_id_and_java_package_before_writing() {
        let temp = tempdir().unwrap();
        let output = temp.path().join("invalid");
        assert!(create_neoforge_project(
            &output, "Bad-Mod", "Bad Mod", "com.example", DEFAULT_NEOFORGE_VERSION, false
        ).is_err());
        assert!(!output.exists());

        assert!(create_neoforge_project(
            &output, "valid_mod", "Valid Mod", "com.example.class", DEFAULT_NEOFORGE_VERSION, false
        ).is_err());
        assert!(!output.exists());
    }

    #[test]
    fn overwrite_replaces_generated_files_but_preserves_unmanaged_files() {
        let temp = tempdir().unwrap();
        let output = temp.path();
        fs::write(output.join("settings.gradle"), "old").unwrap();
        fs::write(output.join("keep.txt"), "keep").unwrap();
        create_neoforge_project(
            output, "example_mod", "Example Mod", "com.example", DEFAULT_NEOFORGE_VERSION, true
        ).unwrap();
        assert!(fs::read_to_string(output.join("settings.gradle")).unwrap().contains("pluginManagement"));
        assert_eq!(fs::read_to_string(output.join("keep.txt")).unwrap(), "keep");
    }
}

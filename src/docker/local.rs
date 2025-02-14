use std::path::Path;
use std::process::ExitStatus;

use super::engine::*;
use super::shared::*;
use crate::cargo::CargoMetadata;
use crate::errors::Result;
use crate::extensions::CommandExt;
use crate::file::{PathExt, ToUtf8};
use crate::{Config, Target};
use atty::Stream;
use eyre::Context;

#[allow(clippy::too_many_arguments)] // TODO: refactor
pub(crate) fn run(
    engine: &Engine,
    target: &Target,
    args: &[String],
    metadata: &CargoMetadata,
    config: &Config,
    uses_xargo: bool,
    sysroot: &Path,
    verbose: bool,
    docker_in_docker: bool,
    cwd: &Path,
) -> Result<ExitStatus> {
    let dirs = Directories::create(engine, metadata, cwd, sysroot, docker_in_docker, verbose)?;

    let mut cmd = cargo_safe_command(uses_xargo);
    cmd.args(args);

    let mut docker = subcommand(engine, "run");
    docker.args(&["--userns", "host"]);
    docker_envvars(&mut docker, config, target)?;

    let mount_volumes = docker_mount(
        &mut docker,
        metadata,
        config,
        target,
        cwd,
        |docker, val| mount(docker, val, ""),
        |_| {},
    )?;

    docker.arg("--rm");

    docker_seccomp(&mut docker, engine.kind, target, metadata, verbose)?;
    docker_user_id(&mut docker, engine.kind);

    docker
        .args(&["-v", &format!("{}:/xargo:Z", dirs.xargo.to_utf8()?)])
        .args(&["-v", &format!("{}:/cargo:Z", dirs.cargo.to_utf8()?)])
        // Prevent `bin` from being mounted inside the Docker container.
        .args(&["-v", "/cargo/bin"]);
    if mount_volumes {
        docker.args(&[
            "-v",
            &format!("{}:{}:Z", dirs.host_root.to_utf8()?, dirs.mount_root),
        ]);
    } else {
        docker.args(&["-v", &format!("{}:/project:Z", dirs.host_root.to_utf8()?)]);
    }
    docker
        .args(&["-v", &format!("{}:/rust:Z,ro", dirs.sysroot.to_utf8()?)])
        .args(&["-v", &format!("{}:/target:Z", dirs.target.to_utf8()?)]);
    docker_cwd(&mut docker, metadata, &dirs, cwd, mount_volumes)?;

    // When running inside NixOS or using Nix packaging we need to add the Nix
    // Store to the running container so it can load the needed binaries.
    if let Some(ref nix_store) = dirs.nix_store {
        docker.args(&[
            "-v",
            &format!("{}:{}:Z", nix_store.to_utf8()?, nix_store.as_posix()?),
        ]);
    }

    if atty::is(Stream::Stdin) {
        docker.arg("-i");
        if atty::is(Stream::Stdout) && atty::is(Stream::Stderr) {
            docker.arg("-t");
        }
    }
    let mut image = image_name(config, target)?;
    if needs_custom_image(target, config) {
        image = custom_image_build(target, config, metadata, dirs, engine, verbose)
            .wrap_err("when building custom image")?
    }

    docker
        .arg(&image)
        .args(&["sh", "-c", &format!("PATH=$PATH:/rust/bin {:?}", cmd)])
        .run_and_get_status(verbose, false)
        .map_err(Into::into)
}

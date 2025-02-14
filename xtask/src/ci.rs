use clap::Subcommand;
use cross::{cargo_command, CargoMetadata, CommandExt};

#[derive(Subcommand, Debug)]
pub enum CiJob {
    /// Return needed metadata for building images
    PrepareMeta {
        // tag, branch
        #[clap(long, env = "GITHUB_REF_TYPE")]
        ref_type: String,
        // main, v0.1.0
        #[clap(long, env = "GITHUB_REF_NAME")]
        ref_name: String,
        target: crate::ImageTarget,
    },
    /// Check workspace metadata.
    Check {
        // tag, branch
        #[clap(long, env = "GITHUB_REF_TYPE")]
        ref_type: String,
        // main, v0.1.0
        #[clap(long, env = "GITHUB_REF_NAME")]
        ref_name: String,
    },
}

pub fn ci(args: CiJob, metadata: CargoMetadata) -> cross::Result<()> {
    let cross_meta = metadata
        .get_package("cross")
        .expect("cross expected in workspace");

    match args {
        CiJob::PrepareMeta {
            ref_type,
            ref_name,
            target,
        } => {
            // Set labels
            let mut labels = vec![];

            labels.push(format!(
                "org.opencontainers.image.title=cross (for {})",
                target.triplet
            ));
            labels.push(format!(
                "org.opencontainers.image.licenses={}",
                cross_meta.license.as_deref().unwrap_or_default()
            ));

            gha_output("labels", &serde_json::to_string(&labels.join("\n"))?);

            let version = cross_meta.version.clone();

            // Set image name
            gha_output(
                "image",
                &crate::build_docker_image::determine_image_name(
                    &target,
                    cross::docker::CROSS_IMAGE,
                    &ref_type,
                    &ref_name,
                    false,
                    &version,
                )?[0],
            );

            if target.has_ci_image() {
                gha_output("has-image", "true")
            }
        }
        CiJob::Check { ref_type, ref_name } => {
            let version = semver::Version::parse(&cross_meta.version)?;
            if ref_type == "tag" {
                if ref_name.starts_with('v') && ref_name != format!("v{version}") {
                    eyre::bail!("a version tag was published, but the tag does not match the current version in Cargo.toml");
                }
                let search = cargo_command()
                    .args(&["search", "--limit", "1"])
                    .arg("cross")
                    .run_and_get_stdout(true)?;
                let (cross, rest) = search
                    .split_once(" = ")
                    .ok_or_else(|| eyre::eyre!("cargo search failed"))?;
                assert_eq!(cross, "cross");
                // Note: this version includes pre-releases.
                let latest_version = semver::Version::parse(
                    rest.split('"')
                        .nth(1)
                        .ok_or_else(|| eyre::eyre!("cargo search returned unexpected data"))?,
                )?;
                if version >= latest_version && version.pre.is_empty() {
                    gha_output("is-latest", "true")
                }
            }
        }
    }
    Ok(())
}

#[track_caller]
fn gha_output(tag: &str, content: &str) {
    if content.contains('\n') {
        // https://github.com/actions/toolkit/issues/403
        panic!("output `{tag}` contains newlines, consider serializing with json and deserializing in gha with fromJSON()")
    }
    println!("::set-output name={tag}::{}", content)
}

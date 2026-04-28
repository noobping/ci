use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn make_fake_podman(dir: &Path) -> PathBuf {
    make_fake_container_runtime(dir, "podman")
}

pub fn make_fake_docker(dir: &Path) -> PathBuf {
    make_fake_container_runtime(dir, "docker")
}

fn make_fake_container_runtime(dir: &Path, name: &str) -> PathBuf {
    let bin_dir = dir.join("bin");
    fs::create_dir_all(&bin_dir).expect("create fake bin dir");
    let log = dir.join(format!("{name}.log"));
    let runtime = bin_dir.join(name);
    fs::write(
        &runtime,
        format!(
            r#"#!/bin/sh
set -eu
log={}
printf '%s\n' "$*" >> "$log"

if [ "$#" -gt 0 ] && [ "$1" = "build" ]; then
  exit 0
fi

if [ "$#" -gt 0 ] && [ "$1" = "run" ]; then
  shift
  repo=
  workdir=/work
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --rm|--network)
        if [ "$1" = "--network" ]; then shift; fi
        shift
        ;;
      -v)
        mount=$2
        case "$mount" in
          *:/work|*:/work:*)
            repo=${{mount%%:/work*}}
            ;;
        esac
        shift 2
        ;;
      -w)
        workdir=$2
        shift 2
        ;;
      -e)
        export "$2"
        shift 2
        ;;
      --platform|--entrypoint|--name)
        shift 2
        ;;
      -d)
        exit 0
        ;;
      *)
        image=$1
        shift
        break
        ;;
    esac
  done

  if [ -n "${{repo:-}}" ]; then
    case "$workdir" in
      /work) cd "$repo" ;;
      /work/*) cd "$repo/${{workdir#/work/}}" ;;
    esac
  fi

  if [ "$#" -ge 3 ] && [ "$2" = "-c" ]; then
    exec "$1" -c "$3"
  fi
  exec "$@"
fi

if [ "$#" -gt 0 ] && [ "$1" = "rm" ]; then
  exit 0
fi

exit 0
"#,
            shell_quote(&log)
        ),
    )
    .expect("write fake container runtime");
    let mut permissions = fs::metadata(&runtime)
        .expect("fake container runtime metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&runtime, permissions).expect("make fake container runtime executable");
    bin_dir
}

pub fn make_fake_flatpak_spawn(dir: &Path, host_bin: &Path) -> PathBuf {
    let bin_dir = dir.join("flatpak-bin");
    fs::create_dir_all(&bin_dir).expect("create fake flatpak bin dir");
    let flatpak_spawn = bin_dir.join("flatpak-spawn");
    fs::write(
        &flatpak_spawn,
        format!(
            r#"#!/bin/sh
set -eu
if [ "${{1:-}}" = "--host" ]; then
  shift
fi
PATH={}:$PATH
export PATH
exec "$@"
"#,
            shell_quote(host_bin)
        ),
    )
    .expect("write fake flatpak-spawn");
    let mut permissions = fs::metadata(&flatpak_spawn)
        .expect("fake flatpak-spawn metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&flatpak_spawn, permissions).expect("make fake flatpak-spawn executable");
    bin_dir
}

pub fn make_fake_shell(dir: &Path) -> PathBuf {
    let bin_dir = dir.join("sh-bin");
    fs::create_dir_all(&bin_dir).expect("create fake sh bin dir");
    let sh = bin_dir.join("sh");
    fs::write(
        &sh,
        r#"#!/bin/sh
exec /bin/sh "$@"
"#,
    )
    .expect("write fake sh");
    let mut permissions = fs::metadata(&sh).expect("fake sh metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&sh, permissions).expect("make fake sh executable");
    bin_dir
}

pub fn path_with_fake_bins(fake_bins: &[PathBuf]) -> String {
    std::env::join_paths(fake_bins)
        .expect("join fake PATH")
        .to_string_lossy()
        .to_string()
}

pub fn path_with_fake_bin(fake_bin: &Path) -> String {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![fake_bin.to_path_buf()];
    paths.extend(std::env::split_paths(&current));
    std::env::join_paths(paths)
        .expect("join PATH")
        .to_string_lossy()
        .to_string()
}

fn shell_quote(path: &Path) -> String {
    let value = path.as_os_str().to_string_lossy();
    let escaped = value.replace('\'', "'\\''");
    format!("'{escaped}'")
}

project(
  'secure-folder',
  version: '0.1.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.3.0',
)

app_id = 'io.github.axel7083.SecureFolder'

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
libexecdir = prefix / get_option('libexecdir')
datadir = prefix / get_option('datadir')

cargo = find_program('cargo')
gocryptfs = find_program('gocryptfs', required: false)

cargo_profile = get_option('buildtype') == 'debug' ? 'debug' : 'release'
cargo_target_dir = meson.project_build_root() / 'cargo-target'
cargo_args = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', cargo_target_dir,
]
if cargo_profile == 'release'
  cargo_args += ['--release']
endif
if get_option('offline')
  cargo_args += ['--offline']
endif

# Binary name -> install directory. Daemons and helpers go to libexecdir,
# user-facing commands to bindir.
binaries = {
  'secure-folder': bindir,
  'secure-folder-ctl': bindir,
  'sf-session': libexecdir,
  'sf-keyd': libexecdir,
  'sf-gvfs-monitor': libexecdir,
}

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: binaries.keys(),
  console: true,
  command: [
    'sh', '-c',
    '"$1" build "${@:3}" && for b in ' + ' '.join(binaries.keys()) + '; do cp "$2/$b" "@OUTDIR@/$b"; done',
    'cargo-build', cargo, cargo_target_dir / cargo_profile,
  ] + cargo_args,
)

foreach name, dir : binaries
  install_data(meson.project_build_root() / name, install_dir: dir, install_mode: 'rwxr-xr-x')
endforeach

subdir('data')

test(
  'cargo-test',
  cargo,
  args: ['test', '--workspace'] + cargo_args,
  timeout: 600,
  is_parallel: false,
)

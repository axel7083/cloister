data_conf = configuration_data()
data_conf.set('libexecdir', libexecdir)
data_conf.set('bindir', bindir)

session_unit = configure_file(
  input: 'systemd/cloister-session.service.in',
  output: 'cloister-session.service',
  configuration: data_conf,
  install_dir: prefix / 'lib' / 'systemd' / 'user',
)
test('check-session-unit', find_program('../tests/check-session-unit.sh'), args: [session_unit])

configure_file(
  input: 'dbus-1/io.github.axel7083.Cloister.Session.service.in',
  output: 'io.github.axel7083.Cloister.Session.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'services',
)

configure_file(
  input: 'dbus-1/io.github.axel7083.Cloister.service.in',
  output: 'io.github.axel7083.Cloister.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'services',
)

desktop_file = configure_file(
  input: 'applications/io.github.axel7083.Cloister.desktop.in',
  output: 'io.github.axel7083.Cloister.desktop',
  configuration: data_conf,
  install_dir: datadir / 'applications',
)

desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('validate-desktop', desktop_file_validate, args: [desktop_file])
endif

install_subdir('icons/hicolor', install_dir: datadir / 'icons')

# System part (key service). Needs root to install.
configure_file(
  input: 'systemd/cloister-keyd.service.in',
  output: 'cloister-keyd.service',
  configuration: data_conf,
  install_dir: prefix / 'lib' / 'systemd' / 'system',
)

configure_file(
  input: 'dbus-1/io.github.axel7083.Cloister.KeyService.system-service.in',
  output: 'io.github.axel7083.Cloister.KeyService.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'system-services',
)

install_data('dbus-1/system.d/io.github.axel7083.Cloister.KeyService.conf',
  install_dir: datadir / 'dbus-1' / 'system.d')
install_data('polkit-1/io.github.axel7083.Cloister.policy',
  install_dir: datadir / 'polkit-1' / 'actions')
install_data('sysusers.d/cloister.conf',
  install_dir: prefix / 'lib' / 'sysusers.d')

xmllint = find_program('xmllint', required: false)
if xmllint.found()
  test('validate-polkit', xmllint, args: ['--noout', files('polkit-1/io.github.axel7083.Cloister.policy')])
  test('validate-dbus-policy', xmllint, args: ['--noout', files('dbus-1/system.d/io.github.axel7083.Cloister.KeyService.conf')])
endif

# GVfs volume monitor (sidebar entry in GIO file managers).
configure_file(
  input: 'dbus-1/io.github.axel7083.Cloister.VolumeMonitor.service.in',
  output: 'io.github.axel7083.Cloister.VolumeMonitor.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'services',
)

configure_file(
  input: 'systemd/cloister-gvfs-monitor.service.in',
  output: 'cloister-gvfs-monitor.service',
  configuration: data_conf,
  install_dir: prefix / 'lib' / 'systemd' / 'user',
)

install_data('gvfs/cloister.monitor',
  install_dir: datadir / 'gvfs' / 'remote-volume-monitors')

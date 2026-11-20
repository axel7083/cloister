data_conf = configuration_data()
data_conf.set('libexecdir', libexecdir)
data_conf.set('bindir', bindir)

configure_file(
  input: 'systemd/sf-session.service.in',
  output: 'sf-session.service',
  configuration: data_conf,
  install_dir: prefix / 'lib' / 'systemd' / 'user',
)

configure_file(
  input: 'dbus-1/io.github.axel7083.SecureFolder.Session.service.in',
  output: 'io.github.axel7083.SecureFolder.Session.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'services',
)

configure_file(
  input: 'dbus-1/io.github.axel7083.SecureFolder.service.in',
  output: 'io.github.axel7083.SecureFolder.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'services',
)

desktop_file = configure_file(
  input: 'applications/io.github.axel7083.SecureFolder.desktop.in',
  output: 'io.github.axel7083.SecureFolder.desktop',
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
  input: 'systemd/sf-keyd.service.in',
  output: 'sf-keyd.service',
  configuration: data_conf,
  install_dir: prefix / 'lib' / 'systemd' / 'system',
)

configure_file(
  input: 'dbus-1/io.github.axel7083.SecureFolder.KeyService.system-service.in',
  output: 'io.github.axel7083.SecureFolder.KeyService.service',
  configuration: data_conf,
  install_dir: datadir / 'dbus-1' / 'system-services',
)

install_data('dbus-1/system.d/io.github.axel7083.SecureFolder.KeyService.conf',
  install_dir: datadir / 'dbus-1' / 'system.d')
install_data('polkit-1/io.github.axel7083.SecureFolder.policy',
  install_dir: datadir / 'polkit-1' / 'actions')
install_data('sysusers.d/secure-folder.conf',
  install_dir: prefix / 'lib' / 'sysusers.d')

xmllint = find_program('xmllint', required: false)
if xmllint.found()
  test('validate-polkit', xmllint, args: ['--noout', files('polkit-1/io.github.axel7083.SecureFolder.policy')])
  test('validate-dbus-policy', xmllint, args: ['--noout', files('dbus-1/system.d/io.github.axel7083.SecureFolder.KeyService.conf')])
endif

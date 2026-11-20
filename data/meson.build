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

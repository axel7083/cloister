data_conf = configuration_data()
data_conf.set('libexecdir', libexecdir)

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

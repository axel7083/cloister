# Built from a source tarball plus a tarball of vendored crates (offline,
# reproducible). Produce both with packaging/build-rpm.sh.

Name:           cloister
Version:        0.1.0
Release:        1%{?dist}
Summary:        Encrypted folders that live in your file manager

# Cloister itself is GPL-3.0-or-later; the rest comes from bundled crates
# (see LICENSE.dependencies, generated at build time).
License:        GPL-3.0-or-later AND Apache-2.0 AND Apache-2.0 WITH LLVM-exception AND MIT AND Unicode-3.0 AND (Apache-2.0 OR MIT) AND (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) AND (MIT OR Unlicense)
URL:            https://github.com/axel7083/cloister
Source0:        %{url}/releases/download/v%{version}/%{name}-%{version}.tar.gz
Source1:        %{url}/releases/download/v%{version}/%{name}-%{version}-vendor.tar.xz

ExclusiveArch:  %{rust_arches}

BuildRequires:  cargo-rpm-macros >= 24
BuildRequires:  meson >= 1.3.0
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(libadwaita-1) >= 1.6
BuildRequires:  desktop-file-utils
BuildRequires:  libxml2
BuildRequires:  systemd-rpm-macros
# For %%check (FUSE tests skip themselves in the build root).
BuildRequires:  gocryptfs
BuildRequires:  fuse3

Requires:       gocryptfs
# fusermount3, setuid: mounts the vaults.
Requires:       fuse3
Requires:       polkit
Requires:       dbus
# The sidebar entry needs GVfs; everything else works without it.
Recommends:     gvfs
# The "cloister" system user comes from %%{_sysusersdir}/cloister.conf;
# rpm creates it on install (automatic sysusers.d handling).

%description
Cloister keeps private files in encrypted folders (gocryptfs) that show up in
the sidebar of GNOME Files and other GIO file managers. A folder unlocks with
its password or with your fingerprint (through polkit), and locks itself when
the screen locks, the computer sleeps or you log out.

%prep
%autosetup -n %{name}-%{version} -a1
%cargo_prep -v vendor

%build
export RUSTFLAGS="%{build_rustflags}"
%meson -Doffline=true
%meson_build
%{cargo_license_summary}
%{cargo_license} > LICENSE.dependencies
%cargo_vendor_manifest

%install
%meson_install

%check
%meson_test

%post
%systemd_post cloister-keyd.service
%systemd_user_post cloister-session.service cloister-gvfs-monitor.service

%preun
%systemd_preun cloister-keyd.service
%systemd_user_preun cloister-session.service cloister-gvfs-monitor.service

%postun
%systemd_postun_with_restart cloister-keyd.service
%systemd_user_postun cloister-session.service cloister-gvfs-monitor.service

%files
%license LICENSE LICENSE.dependencies
%license cargo-vendor.txt
%doc README.md
%{_bindir}/cloister
%{_bindir}/cloister-ctl
%{_libexecdir}/cloister-session
%{_libexecdir}/cloister-keyd
%{_libexecdir}/cloister-gvfs-monitor
%{_unitdir}/cloister-keyd.service
%{_userunitdir}/cloister-session.service
%{_userunitdir}/cloister-gvfs-monitor.service
%{_sysusersdir}/cloister.conf
%{_datadir}/applications/io.github.axel7083.Cloister.desktop
%{_datadir}/dbus-1/services/io.github.axel7083.Cloister.service
%{_datadir}/dbus-1/services/io.github.axel7083.Cloister.Session.service
%{_datadir}/dbus-1/services/io.github.axel7083.Cloister.VolumeMonitor.service
%{_datadir}/dbus-1/system-services/io.github.axel7083.Cloister.KeyService.service
%{_datadir}/dbus-1/system.d/io.github.axel7083.Cloister.KeyService.conf
%{_datadir}/polkit-1/actions/io.github.axel7083.Cloister.policy
%{_datadir}/gvfs/remote-volume-monitors/cloister.monitor
%{_datadir}/icons/hicolor/scalable/apps/io.github.axel7083.Cloister.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.axel7083.Cloister-symbolic.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.axel7083.Cloister-locked-symbolic.svg
%{_datadir}/icons/hicolor/symbolic/apps/io.github.axel7083.Cloister-unlocked-symbolic.svg

%changelog
* Fri Nov 20 2026 axel7083 <42176370+axel7083@users.noreply.github.com> - 0.1.0-1
- First package

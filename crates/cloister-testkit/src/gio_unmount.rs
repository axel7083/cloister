//! `cloister-gio-unmount <vault-id>`: unmounts a vault the way Nautilus's eject
//! button does, through the `GMount` the volume monitor exposes
//! (`g_mount_unmount_with_operation`). `gio mount -u file://…` cannot be used:
//! GIO resolves file paths through the native unix-mount monitor only, so the
//! request would never reach cloister-gvfs-monitor.
//!
//! Exit status: 0 unmounted, 1 GIO error (printed), 2 usage / no such mount.

use std::process::ExitCode;

use gio::prelude::*;

fn main() -> ExitCode {
    let Some(id) = std::env::args().nth(1) else {
        eprintln!("usage: cloister-gio-unmount <vault-id>");
        return ExitCode::from(2);
    };
    let monitor = gio::VolumeMonitor::get();
    let mount = monitor.mounts().into_iter().find(|m| {
        m.volume()
            .and_then(|v| v.identifier(gio::VOLUME_IDENTIFIER_KIND_UUID))
            .is_some_and(|u| u == id)
    });
    let Some(mount) = mount else {
        eprintln!("no mounted volume with uuid {id}");
        return ExitCode::from(2);
    };
    let ctx = gio::glib::MainContext::default();
    let result = ctx.block_on(
        mount
            .unmount_with_operation_future(gio::MountUnmountFlags::NONE, gio::MountOperation::NONE),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

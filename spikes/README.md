# Milestone 0 spikes

Throw-away checks for the risky assumptions of the MVP plan. Delete this folder
once every item is resolved (record the outcome here first).

| # | Assumption | How to check | Outcome |
|---|---|---|---|
| 1 | polkit `org.freedesktop.policykit.owner` lets a non-root service user check other callers | Same pattern as systemd-resolved / timesyncd (`/usr/share/polkit-1/actions/org.freedesktop.resolve1.policy`) | ✅ precedent on F45 |
| 2 | gocryptfs takes the password and master key from a pipe | `./gocryptfs-pipes.sh` | ✅ `-passfile /dev/stdin`, `gocryptfs-xray -dumpmasterkey` (stdin), `-masterkey=stdin`, `-fg` all work |
| 3 | Nautilus 51 shows a non-native remote-monitor volume, custom icon, mount name, `VolumeMount` ignoring the mount op | after milestone 5 prototype | pending |
| 4 | GNOME Shell updates logind `LockedHint` on screen lock | `./locked-hint.sh`, then press Super+L | ✅ flips `yes` on lock, `no` on unlock (F45, Wayland), so no GNOME-specific code is needed |

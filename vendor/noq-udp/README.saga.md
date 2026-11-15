# Saga patch: `noq-udp` EPERM fallback

Vendored from crates.io `noq-udp` 1.0.1.

## Change

On `sendmsg` failure, treat `EPERM` like `EINVAL` when enabling the
no-`IP_TOS` / no-ECN fallback and retry the datagram.

## Why

Android API 29 AVDs use Linux 4.14. Setting the `IP_TOS` cmsg for ECN can
return `EPERM` (`PermissionDenied`) rather than `EINVAL`. Upstream only falls
back on `EINVAL`/`EIO`, so QUIC UDP sends can fail until the fallback engages.

API 35 (kernel 6.6) does not normally hit this path. A cold `-wipe-data` boot
of the API 29 AVD also cleared a wedged emulator network that produced
persistent `EPERM` even without ECN cmsgs.

## Upstream

Prefer dropping this patch once n0/`noq-udp` accepts an equivalent change.

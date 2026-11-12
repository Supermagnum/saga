# Saga call-state icons

Vendored from the [gnupg-hamradio](https://github.com/Supermagnum/gnupg-hamradio)
icon set. License: **AGPL**. Surface AGPL notices in the app's about/licenses
screen before shipping (see saga-spec.md §8, Fallback UX).

## File mapping (§13)

| File | In-call state | Meaning |
|---|---|---|
| `open.svg` | Encryption possible | Handshake in progress; peer key resolves |
| `locked.svg` | Encrypted | Session key active, transcript confirmed |
| `key-error.svg` | Key not found | No resolvable key; neutral "not encrypted" |
| `key.svg` | (supplementary) | Gnupg/Galdra keys loaded in the system |
| `broken.svg` | Downgraded (warning) | Broken padlock; Android vector `ic_saga_padlock_downgraded.xml` |

Paired status text: "Securing…", "Encrypted", "Not encrypted", and the §5a
modal for downgrade (not icon-only).

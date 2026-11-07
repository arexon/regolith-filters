# copperscript

Builds a [Copperscript](https://github.com/arexon/copperscript) plugin into the behavior pack, and adds its native script module and dependencies to the manifest.

```sh
regolith install github.com/arexon/regolith-filters/copperscript==0.1.0
```

Put the plugin crate in `packs/data/copperscript`, and write `packs/BP/manifest.json` without a script module.

## Settings

| Setting | Default | Meaning |
|---|---|---|
| `crate` | `"copperscript"` | The crate's folder inside the data folder |
| `profile` | `"release"` | The cargo profile |

## Requirements

`cargo` with the `x86_64-pc-windows-msvc` target. Outside Windows, it needs a cross linker and the Windows SDK as well, such as `lld-link` and [xwin](https://github.com/Jake-Shadle/xwin).

Runs on Windows x86-64, macOS, and Linux x86-64.

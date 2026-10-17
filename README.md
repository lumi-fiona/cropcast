# cropcast

Share part of your screen on Discord, pixel for pixel, with full system audio.

Discord shares a whole screen or one window, never part of a screen. On a 5120×1440 ultrawide, the whole screen reaches viewers as a letterboxed strip too small to read, and a window share follows one app and pauses when it's minimised. cropcast lets you pick an Area of the screen and share exactly that, as a normal Discord Screen share.

<!-- Lumi records this with `cropcast.exe --demo`, which leaves the Border visible to screen recorders. -->
![Hold Ctrl+Alt, drag the Area, and the Discord viewer shows only the Area](docs/demo.gif)

## How it works

1. cropcast adds a Virtual monitor to Windows, exactly the size of the Area.
2. It captures the real monitor and copies the Area onto the Virtual monitor on the GPU, 60 times a second.
3. Discord shares the Virtual monitor like any screen, so the cursor and system audio come along.

The full pipeline, and the dead ends on the way, are in [docs/architecture.md](docs/architecture.md).

## Download and use

Windows 11 only.

1. Download `cropcast.exe` from [Releases](https://github.com/lumi-fiona/cropcast/releases/latest). It's portable: no installer, nothing to unzip. Run it and it sits in the tray.
2. Turn on **Virtual monitor** in the tray menu. The first time, Windows asks once for admin rights to install the Driver (the virtual display driver). After that, cropcast never asks again.
3. In Discord, start a Screen share and pick the new screen.
4. Hold **Ctrl+Alt** for Edit mode: the Border shows the Area and shades the rest of the screen. Drag an edge or a corner to resize, drag inside to move, and let go to commit. Resizes snap near 16:9 and 21:9 (3440×1440), and the size is shown live.

The tray menu:

- **Virtual monitor**: on adds the screen and starts the mirror; off removes it and leaves the desktop as it was.
- **Start with Windows**
- **Reset Area to 2560×1440 center**
- **Show Border briefly**: shows where the Area is without entering Edit mode.
- **Remove driver…**: uninstalls the Driver and its certificate (`cropcast.exe --uninstall` does the same).
- **Quit**

The Area is saved between runs. For a sharp stream, use Discord Nitro with the Source quality preset; without it, Discord scales the stream down.

## Known limits

- Anything that lands on the Virtual monitor (a stray window, the cursor) is seen by viewers. It sits off the bottom-right corner of the real screen, so this rarely happens by accident.
- Turning the Virtual monitor off ends the Discord share; after turning it back on, pick the screen again in Discord.
- A resize glitches the stream for a moment. Discord keeps sharing, because it's the same monitor.
- Exclusive-fullscreen games may mirror black; run them borderless windowed.
- SDR only.
- If you hear yourself echo through the share, an audio router (Sonar, Voicemeeter, FxSound) is re-playing the call. Point Discord's output straight at your real device.
- The Driver is signed with cropcast's own certificate, which cropcast trusts machine-wide when it installs the Driver. **Remove driver…** takes it out again.

More cases (sleep, resolution changes, the Driver disappearing) are in [docs/limits.md](docs/limits.md).

## Build from source

You need Rust (stable, 2024 edition) on Windows.

```
cargo build --release
```

That gives `target/release/cropcast.exe` with no Driver inside: it uses a SudoVDA from the fork below if one is already installed, but can't install one itself. CI builds the Driver from the fork with the WDK, signs it, and builds the released exe with `--features driver`, which embeds it; see [.github/workflows/build.yml](.github/workflows/build.yml).

## Credits

- [SudoVDA](https://github.com/SudoMaker/SudoVDA) by SudoMaker (MIT), the virtual display driver behind the Virtual monitor.
- [lumi-fiona/SudoVDA](https://github.com/lumi-fiona/SudoVDA), cropcast's fork, which adds `IOCTL_UPDATE_MODE` so the Virtual monitor resizes in place.
- [Apollo](https://github.com/ClassicOldSong/Apollo), whose SudoVDA integration showed the IOCTL call sequence and how a self-signed display driver gets installed.

## License

[MIT](LICENSE)

# Known limits

Notes for the README's "Known limits" section ([Harden the Area against real-world edge cases](https://github.com/lumi-fiona/cropcast/issues/21)).

## Exclusive fullscreen games

cropcast copies the real monitor through Windows.Graphics.Capture (WGC), which captures what the desktop compositor draws. A game in true exclusive fullscreen skips the compositor, so the mirror may go black or freeze while it runs. Microsoft says capture in that mode "is not guaranteed", and Discord's own support page says WGC "does not work for full screen exclusive (FSE) games".

Most modern games that offer "Fullscreen" really run borderless behind the scenes (Windows' fullscreen optimizations), and those mirror fine. Borderless or windowed fullscreen always mirrors.

While the Virtual monitor is on, cropcast asks Windows every 2 s whether a game is in exclusive fullscreen (`SHQueryUserNotificationState`, the same test Discord uses) and warns once per run. Windows also says yes for games it quietly runs borderless, so the warning says "may". Windows holds notifications back while a game is fullscreen, so the warning can show up after the game closes.

A game that changes the real monitor's resolution when it goes fullscreen makes cropcast refit the Area and resize the Virtual monitor (see below), so viewers see the stream change size.

## The real monitor changes resolution, refresh rate or scaling

- **Resolution:** cropcast refits the Area to the new size. It uses the saved Area, moved onto the screen, or the default Area (2560×1440 centered, shrunk to fit) if the saved one is too big. The refit isn't saved, so the saved Area comes back when the resolution does. The Virtual monitor is resized and moved back to the real monitor's bottom-right corner, and the capture restarts. Discord keeps sharing unless Windows refuses the new size, in which case the Virtual monitor is re-added and Discord's share pauses.
- **Refresh rate:** only the capture restarts. Below 60 Hz, the stream drops to the monitor's rate.
- **Scaling (DPI):** nothing changes. The Area is in physical pixels, and so is everything cropcast draws.
- **Holding Ctrl+Alt while the resolution changes:** Edit mode keeps the old screen size until the keys are released, so the committed Area can hang off the screen. Viewers see black for the part that's off. Moving the Area again fixes it.

## The real monitor goes away, or the PC sleeps

If the real monitor turns off or unplugs and Windows makes the Virtual monitor its main screen, cropcast turns the Virtual monitor off and says so. Turn it on again from the tray when the real monitor is back.

Across sleep, the Driver's watchdog shouldn't fire, because both it and cropcast pause. On resume cropcast checks that the Virtual monitor is still there and restarts the capture. If Windows dropped the Virtual monitor during sleep, cropcast turns it off and says so. Discord usually leaves the call during sleep anyway, so the share has to be picked again.

## The Driver is removed while the Virtual monitor is on

If the Driver goes (for example, removed by hand in Device Manager), the Virtual monitor goes with it. cropcast turns off and says the Driver went away. Turning the Virtual monitor on again installs it. While the Virtual monitor is on, cropcast keeps the Driver open, so Device Manager may ask for a restart instead of removing it at once. Use the tray's "Remove driver…" instead.

## Discord echo through speakers

From [Keep Discord's own audio out of the share](https://github.com/lumi-fiona/cropcast/issues/8). cropcast needs no code for this.

Stock Discord keeps its own call audio out of a screen share's sound. It captures share audio per process, and a whole-screen share leaves Discord's own processes out. It doesn't capture the output device, so which device Discord plays on doesn't matter by itself.

Viewers hear themselves when an audio router plays Discord's sound again from its own process: SteelSeries Sonar, Voicemeeter, FxSound, or VB-Cable "listen" setups. That copy doesn't come from Discord, so it gets shared. The fix is to set Discord's Output Device straight to the real headset or speakers, or to mute Discord in the router's mix. A virtual cable makes it worse. Discord has no setting for it.

Not confirmed on current Discord builds: that whole-screen shares really leave Discord out. That comes from Discord engineers' design notes and user reports.

# Keeping Discord's own audio out of the share

Research for [#8](https://github.com/lumi-fiona/cropcast/issues/8), part of map [#1](https://github.com/lumi-fiona/cropcast/issues/1). Researched 2026-10-16.

Question: how does Discord's Windows desktop client capture Screen-share audio, does it already leave out Discord's own sound, and which fix is least effort?

Evidence levels:

- **Confirmed**: Microsoft docs, or Discord engineers speaking for Discord in public.
- **Reported**: users, third-party client mods, vendor help pages. Credible, not first-party.
- **Inference**: my reasoning. Not verified.

## Short answer

1. Discord does not tap the default output device. It uses Windows **process loopback** (`ActivateAudioInterfaceAsync` with `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`). A window share **includes** the shared app's process tree. A whole-screen share is meant to capture **everything except Discord's own process tree** (Confirmed for the API and the design; that current builds run exclude mode is Reported + Inference).
2. So stock Discord should **already keep its own call audio out** of a Screen share. Echo happens when **another process re-plays Discord's audio**: SteelSeries Sonar, Voicemeeter, FxSound, VB-Cable "listen" setups and similar. That process is not in Discord's tree, so its copy of the call gets captured (Reported, several independent sources).
3. **Least-effort fix: nothing, if Lumi's audio goes straight to a real device.** If echo shows up, the fix is in the audio router, not in Discord or cropcast: point Discord's output straight at the real headset/speakers (not the router's virtual device), or mute Discord inside the router's mix.
4. **cropcast needs no code.** It makes no sound, and it can't change how Discord captures.

This corrects the earlier note in `docs/research/discord-virtual-monitor.md` (branch `research/discord-virtual-monitor`), which guessed "loopback of the default output device" and that echo is inherent.

## 1. How Discord captures share audio

### Confirmed

- A Discord engineer who "worked on soundshare in Discord for both Windows and X11", Aug 2023: "`ActivateAudioInterfaceAsync` with the `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK` activation type is what we use on sufficiently recent versions of Windows." Include-tree mode is for window shares. Exclude-tree mode "is helpful for streaming full-desktop audio since we want to exclude Discord's audio from the stream to avoid feedback (though we don't currently have soundshare hooked up for whole-screen case)." ([xdg-desktop-portal #957, mikepavone](https://github.com/flatpak/xdg-desktop-portal/issues/957#issuecomment-1662782982))
- Another Discord engineer, Mar 2025, describing what Discord wants from Linux: "process-and-children for windowed share, or **all-minus-process-and-children for full-screen**". ([same issue, rcombs](https://github.com/flatpak/xdg-desktop-portal/issues/957#issuecomment-2708716835))
- Discord engineering blog: audio comes from OS audio APIs, "from the shared process and all its children" for app shares. ([How It All Goes Live](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology), Mar 2024)
- Windows API: `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE` means "render streams from the specified process and its child processes are excluded". Needs build 20348+, so every Win11 build has it. ([PROCESS_LOOPBACK_MODE](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ne-audioclientactivationparams-process_loopback_mode), [AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS](https://learn.microsoft.com/en-us/windows/win32/api/audioclientactivationparams/ns-audioclientactivationparams-audioclient_process_loopback_params))
- The API takes **one** target process tree per capture. Microsoft's sample: "the capture is not tied to a specific audio endpoint". ([ApplicationLoopback sample](https://github.com/microsoft/windows-classic-samples/tree/main/Samples/ApplicationLoopback))

### Reported

- Whole-screen audio arrived on Windows around Jan 2024, after the engineer's 2023 comment. (See the earlier research note.)
- A Vencord plugin author logged Discord's own `setGoLiveSource` calls: whole screen sends `soundshareId: 1`, a window share sends the app's PID. `1` is a sentinel for "everything", handled in native code. ([soundshareSource.desktop](https://github.com/jimman0I/soundshareSource.desktop), Aug 2026) The same README claims Discord has no exclude mode in its binary. That claim is weak: enum names don't appear in compiled code, and the mode is just an integer.
- `discord_voice` (the module that plays the call) runs inside Discord's renderer process, a child of `Discord.exe`. So an exclude-tree capture rooted at Discord covers the call. ([FRD_GOLIVE PR #26](https://github.com/BirdRa1n/FRD_GOLIVE/pull/26), a third-party Go Live mod, Sep 2026)

### Inference

- Putting it together: stock Discord's whole-screen share most likely runs exclude-tree on itself. Pure default-endpoint loopback wouldn't fit what users see (next section). Not verified in current builds; the hands-on test below settles it.

## 2. Where the echo actually comes from

### Reported

- Reddit thread asking exactly this question (May 2025). Replies: "Screen sharing your entire desktop shouldn't stream the Discord VC audio", "likely an issue with any audio processing software", "solved this by uninstalling Steelseries.gg", "caused by sonar", "change the output device to your headset and it should be fixed". ([r/discordapp](https://www.reddit.com/r/discordapp/comments/1kjfvel/how_to_prevent_screenshare_from_also_sharing/))
- SteelSeries has a support page for this exact problem, "Setup Sonar to Stream with Discord Without Echo": route Discord's output to the Sonar Chat device and keep it out of the stream mix. ([SteelSeries support](https://support.steelseries.com/hc/en-us/articles/35145998503181-Setup-Sonar-to-Stream-with-Discord-Without-Echo); page blocks scrapers, summary from search results.) Community tools exist that auto-mute Discord's session inside Sonar's device. ([preg26](https://github.com/preg26/Steelseries-Sonar-Fix-Discord-Sharing-Screen-Microphone), [ayment](https://github.com/ayment/Fix-discord-echo))
- The soundshareSource README explains the mechanism: FxSound, Voicemeeter, Sonar, Nahimic, VB-Cable set a virtual device as default, then **their own process** re-renders the mix to the real speakers. Discord's whole-screen capture grabs that second copy too.
- [DiscordScreenshareLoopbackShutup](https://github.com/SKProCH/DiscordScreenshareLoopbackShutup) targets the same Voicemeeter case.
- The Feb 2024 Discord feedback post reporting echo ([link](https://support.discord.com/hc/en-us/community/posts/21594139117847-Screen-sharing-audio)) doesn't say whether a router was in use.

### Inference

- Echo means some process outside Discord's tree is playing the call. With audio going straight from Discord to a real device, no such process exists.

## 3. Which fixes work

| Fix | Works? | Why |
| --- | --- | --- |
| Do nothing (no audio router) | Expected yes | Discord already leaves out its own tree. Needs the hands-on check. |
| Discord's Output Device set to the real headset/speakers | Yes, when a router is the cause | Discord's audio then never passes through the router's process. Reported fix. |
| Windows "App volume and device preferences" for Discord | Same as above, at best | Same effect as Discord's own setting. Discord's own setting is simpler (Inference). |
| Moving Discord to a *different* device, otherwise | Doesn't matter | Process loopback isn't tied to an endpoint, so the device choice doesn't change what's captured. Only bypassing a router helps (Inference from the API docs). |
| Virtual audio cable | No, often the cause | A cable plus "listen"/re-render adds exactly the outside process that leaks the call. |
| A Discord setting or flag | None exists | No toggle found. Only "share system audio" on/off. |
| cropcast code | Not needed | cropcast makes no sound. It could only fight Discord's capture with a client mod, which is out of scope. |

Fallback if it ever still echoes: share without system audio. Or use Discord's window share for the one app whose sound matters (include-tree, Confirmed).

## 4. Hands-on checks (open)

1. **No-router test:** Windows default output and Discord's output both on the real device; speakers, not headphones. Join a call with a second device, share the Virtual monitor with system audio, play music. The viewer should hear the music and **not** their own voice.
2. **Router inventory:** check for Sonar, Voicemeeter, FxSound, Nahimic, VB-Cable or similar on Lumi's PC (tray, `sndvol` device list). If present and test 1 echoes, set Discord's output straight to the real device and retest.
3. If echo appears with no router, process loopback exclusion isn't running in current builds. Then try Discord's output on a separate device and record the result here.

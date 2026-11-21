# SoundSnap

**English** · [Русский](README.ru.md)

<img src="src/soundsnap.png" alt="SoundSnap logo" width="48" height="48">

SoundSnap is a lightweight Windows utility for switching between speakers, headphones, and other audio outputs with one click in the system tray or a global keyboard shortcut. Pick which devices to cycle through and keep every other output available for manual selection.

## Download

Download **SoundSnap.exe** from the [latest release](https://github.com/faqbooooy/SoundSnap/releases/latest) and run it. The app is built for Windows x64 and requires no installer. If you don't see the icon, check the hidden icons in the Windows system tray.

## Controls

| Action | Result |
| --- | --- |
| Left-click the tray icon | Switch to the next output in your quick-switch list |
| `Ctrl + Alt + F9` | Switch outputs with the default shortcut. You can change it in Settings |
| Hover over the tray icon and scroll | Show the volume indicator and adjust the level |
| Middle-click the tray icon | Mute or unmute the current output |
| Right-click → **Open** | Open the device and volume panel |

The main panel shows the active output and the next one in the cycle. Open **All devices** to select any connected output manually, or use **Edit list** to choose which devices the tray click and shortcut cycle through. Changes are saved immediately.

SoundSnap remembers the volume for each output. When switching through the app, it applies the saved level to the destination before routing audio there. If the level cannot be applied, SoundSnap keeps the current output.

## Mixer and audio profiles

Open **Mixer** from the main panel or the tray menu to adjust or mute individual audio sessions on the current output. Apps such as Discord can have more than one session, so they may have separate sliders.

Choose **Profiles → Create profile** to start with your current audio settings. Give it a name, choose the apps it should control, and adjust their levels or mute states. Each profile can have a shortcut, a backup output with its own volume, or a mute option when the main output disconnects. You can also choose whether to return to the main output when it reconnects.

Profile rules apply at the app level. If an app has multiple audio sessions, the profile applies the same level to all of them.

## Settings and updates

SoundSnap follows the Windows display language by default. It uses Russian for Russian Windows and English for other display languages. To override it, choose **English** or **Русский** under **Settings → Language**. Changes apply immediately.

Settings include light, dark, and system themes, a custom keyboard shortcut, and optional startup with Windows. Preferences and profiles are stored in `settings.json` and `profiles.dat` under `%APPDATA%\SoftAudioSwitch`. Device choices and volume levels are stored in the current user's Windows registry. Moving the EXE to another PC does not transfer these settings.

To update, select **Exit** from the tray menu and replace the EXE in the same folder. If startup is enabled and you want to move the EXE, turn startup off before moving it and re-enable it from the new location.

## Build from source

SoundSnap is written in Rust and uses native Windows APIs. See [BUILD.md](BUILD.md) for build commands and the output path.

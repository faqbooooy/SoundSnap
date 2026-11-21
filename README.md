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

Open **Mixer** from the main panel or the tray menu to adjust or mute individual audio sessions. By default it follows the Windows output. Click **Output** in the mixer to view sessions on another connected device. Apps such as Discord can have more than one session, so they may have separate sliders.

Choose **Profiles → New profile** to start with your current output and master volume. Click a profile tile to activate it or use the pencil to edit. **Apply now** previews the profile without saving it.

The editor has four sections: primary output, applications, disconnection, and activation. Use **Running**, **Popular**, or **Choose EXE** to add an app. **Popular** checks your PC for a matching executable. Use **Copy mixer levels** to import current volume and mute states. Set separate volume limits for your primary and backup outputs, or mute audio if the primary device disconnects.

In **Profile activation**, you can set a keyboard shortcut or choose an app from the same picker to activate the profile while it runs. You can require the primary output to be connected. When multiple rules match, the profile with the highest priority wins. Enable **Restore previous audio** to return to your previous output and levels when the last applicable rule ends. Manually switching outputs or profiles overrides automation for apps already running.

Profile rules apply at the app level. If an app has multiple audio sessions, the profile applies the same level to all of them.

## Settings and updates

SoundSnap follows the Windows display language by default. It uses Russian for Russian Windows and English for other display languages. To override it, click **RUS** or **ENG** at the top of **Settings**. Changes apply immediately.

Settings include light, dark, and system themes, a custom keyboard shortcut, and optional startup with Windows. Preferences and profiles are stored in `settings.json` and `profiles.dat` under `%APPDATA%\SoftAudioSwitch`. Device choices and volume levels are stored in the current user's Windows registry. Moving the EXE to another PC does not transfer these settings.

To update, select **Exit** from the tray menu and replace the EXE in the same folder. If startup is enabled and you want to move the EXE, turn startup off before moving it and re-enable it from the new location.

## Build from source

SoundSnap is written in Rust and uses native Windows APIs. See [BUILD.md](BUILD.md) for build commands and the output path.

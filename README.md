# Charpick

Charpick is a small picker for accented letters and symbols, styled like the Windows emoji panel (Win+.). It runs on Windows 10 and 11, and on Linux with KDE Plasma, where it looks like a Plasma app.

<p>
  <img src="docs/win11-dark.png" alt="Charpick, Windows 11 style, dark theme" width="270">
  <img src="docs/win10-light.png" alt="Charpick, Windows 10 style, light theme" width="258">
  <img src="docs/kde-dark.png" alt="Charpick on KDE Plasma, Breeze Dark" width="255">
</p>

- Press **Win+L** (Windows) or **Meta+,** (KDE Plasma) in any text field. On Windows the picker opens next to the text cursor: to its right, or to its left when there is no room. On Plasma it opens in the middle of the screen.
- **Italian** comes first, then **Romanian**, German, French, Spanish and Portuguese. Catalan, Dutch, the Nordic, Central European and Baltic languages, Turkish, Greek, Vietnamese and more follow, then punctuation, currency, math and other symbols.
- **Type to search** by:
  - letter (`s`)
  - name (`e grave`, `cedilla`, `euro`)
  - language, in English, in the language itself or by code (`romanian`, `română`, `ro`)
- **Arrow keys + Enter** pick a character. Enter types it and closes the picker.
- **Hold Shift** for uppercase. By default, Caps Lock also gives uppercase.
- It matches the **Windows 10 or Windows 11** look automatically. On Plasma it uses the **Breeze** look with your color scheme, accent color and font. You can change the style in the tray menu or the settings.
- The shortcut can be changed from the tray menu (**Change shortcut…**): press the new combination and it is saved.
- On Windows it is a single 0.4 MB native executable, with no runtime and no background polling. While hidden it uses no CPU.

Linux has its own section: [Linux (KDE Plasma)](#linux-kde-plasma). The sections up to it are about Windows.

## Install

Run `Charpick-Setup.exe`. No administrator rights are needed. The installer offers two options:

- **Open Charpick with Win+L.** Windows asks for administrator approval once (see below).
- **Start Charpick when I sign in.**

Windows SmartScreen may warn about an unknown publisher, because the build is not code-signed. Choose **More info → Run anyway**.

To uninstall, use *Settings → Apps*. Uninstalling gives Win+L back to Windows and removes the startup entry and the settings. It also asks for administrator approval to undo the one-time permission. If you decline, the leftover permission is harmless: it only lets your own account change your own "disable lock" setting.

## Using it

| Key | Action |
| --- | --- |
| Win+L | Open or close the picker |
| Typing | Search |
| ← → ↑ ↓, Page Up/Down, Home/End | Move the selection |
| Tab / Shift+Tab | Next / previous language |
| Enter | Type the character and close |
| Shift (held) | Uppercase letters |
| Backspace / Ctrl+Backspace | Delete a character / a word of the search |
| Delete | Clear the search |
| Esc | Clear the search, then close |
| **Ctrl+Win+L** | Lock the PC |

- Clicking a character types it and leaves the picker open. To have clicks close it too, set `close_on_click = on`.
- Mouse wheel scrolls; the tabs at the bottom jump to a language.
- Clicking anywhere else, or switching windows, closes the picker.
- The picker never takes the keyboard focus away from your app, just like the emoji panel. Your text cursor and selection stay where they were.

The tray icon menu has:

- The current shortcut, and Change shortcut…
- Lock this PC (or *Let Win+L open Charpick…* until the setup below is done)
- Style (Automatic / Windows 10 / Windows 11 / KDE Plasma)
- Theme (Automatic / Light / Dark)
- Start with Windows
- Clear recently used
- Edit settings…
- How to use
- Quit Charpick

## How Win+L is taken over

Windows handles Win+L itself, before any app can see the key. The only supported way to stop it is the per-user policy `DisableLockWorkstation`. Charpick sets that policy only while it is needed:

- It is set the moment you press the Win key, so Win+L opens the picker.
- It is cleared after 30 seconds without keyboard or mouse input, before sleep, when the screen turns off, when the session locks and when Charpick exits.

So automatic locks keep working: the idle timeout, locking on resume, and Dynamic Lock. *Ctrl+Alt+Del → Lock* and *Start → Lock* are hidden only from the moment you press the Win key until the PC has been idle for 30 seconds.

To lock the PC, press **Ctrl+Win+L** or choose **Lock this PC** in the tray menu. Locking from the Start menu also works once the 30 seconds have passed.

That policy key can normally be changed only by administrators. The one-time setup (installer option or tray menu → *Let Win+L open Charpick…*) lets your own account change that single value. The change applies to `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\System`.

If you skip the setup, Win+L keeps locking the PC and the picker cannot open. In that case, either run the setup from the tray menu or pick another shortcut: tray menu → *Change shortcut…*, or `hotkey = Ctrl+Alt+Space` in the settings.

If your organization already disables locking through policy, Charpick detects that. Win+L then simply opens the picker.

## Settings

The tray menu's **Edit settings** opens `%APPDATA%\Charpick\settings.ini`. Changes apply the next time the picker opens.

| Setting | Values | Default |
| --- | --- | --- |
| `style` | `auto`, `win10`, `win11`, `kde` | `auto` (detects Windows 10 or 11) |
| `theme` | `auto`, `light`, `dark` | `auto` (follows Windows) |
| `backdrop` | `auto`, `acrylic`, `solid` | `auto` (acrylic when transparency effects are on) |
| `hotkey` | e.g. `Win+L`, `Ctrl+Alt+Space`, `Win+Shift+K`, or `none` | `Win+L` |
| `lock_hotkey` | a shortcut, or `none` | `Ctrl+Win+L` |
| `order` | section codes; `*` means "everything else" | `it, ro, de, fr, es, pt, *` |
| `recent_count` | 0–64 | 16 |
| `close_on_click` | `on` / `off` | `off` |
| `caps_lock_uppercase` | `on` / `off` | `on` |
| `gpu` | `on` / `off` (CPU rendering is lighter for a window this small) | `off` |

Section codes:

- Languages: `it ro de fr es pt ca nl sv no da fi is pl cs sk hu hr sl tr et lv lt sq mt cy ga eo vi el`
- Symbols: `punct currency math numbers misc`

## Limitations

- Windows does not let a normal app type into **apps running as administrator** (UIPI). The picker opens over them, but the character is not inserted.
- The text cursor position comes from accessibility APIs. Apps that do not report it get the picker next to the mouse pointer instead.
- While the picker is open it reads the keyboard through a low-level hook, as the emoji panel does. Keys that are not meant for the picker close it and pass through: shortcuts with Ctrl, Alt or Win.

## Linux (KDE Plasma)

Charpick runs on **KDE Plasma 6 in a Wayland session**. It uses KWin to show the picker above other windows and to type the character, and Plasma's shortcut service for its shortcut.

### Install

Building needs Rust ([rustup.rs](https://rustup.rs)), `pkg-config`, and the development files for cairo, pango and libxkbcommon:

| Distribution | Packages |
| --- | --- |
| Arch, CachyOS, Manjaro, EndeavourOS | `cairo pango libxkbcommon pkgconf` (usually installed already) |
| Debian, Ubuntu, KDE neon, Kubuntu | `pkg-config libcairo2-dev libpango1.0-dev libxkbcommon-dev` |
| Fedora, openSUSE | `pkgconf cairo-devel pango-devel libxkbcommon-devel` |

Then, in the source folder:

```bash
sh installer/install-linux.sh
```

The script builds Charpick and installs it for your account only, with no root rights:

- the program, `~/.local/bin/charpick`
- an entry in the application launcher, `~/.local/share/applications/charpick.desktop`
- start at login, `~/.config/autostart/charpick.desktop`. Leave it out with `--no-autostart`, or turn it off later in the tray menu.

Then it starts Charpick, which greets you with a notification. Run the script again to update; it keeps your settings and your *Start at login* choice.

To uninstall, run `sh installer/install-linux.sh --uninstall`. This removes the program, the launcher entry, the login entry and the shortcut. Add `--purge` to delete the settings too.

### The shortcut

The default is **Meta+,** (Meta is the Windows/Super key, `,` is the comma key). Charpick registers it as a Plasma global shortcut, so it works in every app.

To change it:

- **Tray menu → Change shortcut…** Then hold Ctrl, Alt or Meta and press a key. Esc cancels. A combination that another shortcut already uses is refused, and the old one stays. The same prompt opens with `charpick --change-shortcut`, or by right-clicking Charpick in the application launcher.
- **System Settings → Keyboard → Shortcuts → Charpick.**
- **settings.ini:** `hotkey = Ctrl+Alt+Space`. `hotkey = none` means no shortcut; open the picker from the tray icon instead.

The three stay in sync: a change in one place shows up in the others.

### Using it

The keys are the same as on Windows (see [Using it](#using-it)), with Meta+, in place of Win+L. Some things differ:

- The picker opens in the middle of the screen. Wayland does not tell an app where another app's text cursor is.
- The picker takes the keyboard while it is open. **Enter** or a **click** gives the keyboard back to your app, types the character there and closes the picker. Clicking outside the picker or pressing Esc closes it.
- Characters missing from your keyboard layout are typed too.
- Clicking the tray icon opens and closes the picker.

The tray menu has: Open Charpick, Change shortcut…, Style, Colors, Start at login, Clear recently used, Edit settings…, How to use, and Quit.

### Look

- **Style → Automatic** (or `style = kde`) draws the picker like a Breeze dialog. It uses the colors of your Plasma color scheme, its accent color and the Plasma font. A new color scheme applies the next time the picker opens. The Windows 10 and Windows 11 styles are available too.
- **Colors** follows the color scheme, or forces light or dark. Forced light or dark uses Breeze colors with your accent color.
- The background behind the picker is blurred when KWin supports it. Set `backdrop = solid` for a plain one.
- Scaled displays, including fractional scaling such as 125% or 150%, are drawn sharp.

### Settings

The tray menu's **Edit settings…** opens `~/.config/charpick/settings.ini`. Changes apply within a few seconds. The settings are the ones in [Settings](#settings), except:

- `style` defaults to `auto`, which means the KDE Plasma look.
- `backdrop` is `auto`, `blur` or `solid`.
- `hotkey` defaults to `Meta+,`.
- `lock_hotkey`, `close_on_click` and `gpu` are Windows only.

### Command line

```text
charpick                     start Charpick (or greet, when it is already running)
charpick --toggle            open or close the picker
charpick --show              open the picker
charpick --change-shortcut   choose a new shortcut
charpick --quit              stop the running Charpick
charpick --style auto|kde|win11|win10   (also --theme, --backdrop) override the settings for one run
charpick --log               print what happens to the terminal
```

Only one Charpick runs per session. Starting it again passes the option to the running one.

### Other desktops

KDE Plasma is the supported desktop. X11 sessions and GNOME do not work: Charpick needs the Wayland layer-shell protocol, which GNOME does not offer. Other Wayland compositors with layer-shell, such as Sway or Hyprland, are untested. On them, Charpick should start with two differences:

- Bind `charpick --toggle` to a key in the compositor's settings.
- The character is copied to the clipboard instead of typed. A notification tells you to press Ctrl+V.

## Building from source

On Linux, `installer/install-linux.sh` builds and installs Charpick ([Linux (KDE Plasma)](#linux-kde-plasma)). `cargo build --release` alone builds `target/release/Charpick`.

On Windows, the requirements are:

- Rust (stable, MSVC toolchain)
- Inno Setup 6.3 or later, for the installer

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File installer/build.ps1
```

This builds `target\release\Charpick.exe` and `dist\Charpick-Setup.exe`.

- `tools/make-icons.ps1` regenerates the icons in `assets/`.
- `cargo test` runs the unit tests (catalog, search, settings parsing).
- `cargo run --release --features snapshot -- --snapshot out.png --style win10 --theme dark --query ro` renders the picker to a PNG without showing it. This works on both systems. Add `--style kde` for the Plasma look, or `--prompt` for the shortcut prompt.

Command-line options on Windows:

- `--demo` opens the picker right away.
- `--change-shortcut` opens the shortcut prompt.
- `--style`, `--theme` and `--backdrop` override the settings for one run.
- `--quit` closes the running copy.
- `--log` replaces the running copy with one that writes a diagnostic log to `%APPDATA%\Charpick\debug.log`. The log records only modifier keys, the shortcut keys and the picker's own events, never what you type. The next sign-in starts Charpick without the log again.

Code layout:

| File | Contents |
| --- | --- |
| `src/app.rs` | Tray icon, windows, message handling |
| `src/picker.rs` | Layout, drawing (through the `Canvas` trait in `src/gfx.rs`), keyboard and mouse |
| `src/d2d.rs` | Direct2D/DirectWrite drawing |
| `src/hook.rs` | Low-level keyboard/mouse hooks, typing with `SendInput` |
| `src/caret.rs` | Text cursor position (Win32 caret → MSAA → UI Automation) |
| `src/diag.rs` | The opt-in diagnostic log (`--log`) |
| `src/lockpolicy.rs` | The Win+L policy and its one-time setup |
| `src/data.rs` | Character catalog and search |
| `src/theme.rs`, `src/settings.rs`, `src/util.rs` | Look (Windows 10/11 and KDE Plasma), settings, helpers |
| `src/linux/mod.rs` | Linux: start-up, commands, keeping the shortcut in sync with Plasma |
| `src/linux/wl.rs` | Linux: the Wayland panel (layer-shell), keyboard and mouse, typing through KWin |
| `src/linux/paint.rs` | Linux: Cairo/Pango drawing |
| `src/linux/dbus.rs` | Linux: Plasma's shortcut service (KGlobalAccel) and notifications |
| `src/linux/kde.rs` | Linux: reading the Plasma color scheme and font |
| `src/linux/tray.rs`, `src/linux/ipc.rs` | Linux: tray icon and menu; one Charpick per session |

## License

Charpick is free software, released under the GNU General Public License v3 or later. See [`LICENSE`](LICENSE).

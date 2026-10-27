#!/bin/sh
# Installs Charpick for the current user on Linux (KDE Plasma on Wayland).
#
#   sh install.sh                       install, start at login, start now
#   sh install.sh --no-autostart        the same, without starting at login
#   sh install.sh --uninstall           remove Charpick (keeps the settings)
#   sh install.sh --uninstall --purge   also remove the settings
#
# Nothing is written outside your home directory and no root rights are needed.
#
# In the release download (charpick-linux-x86_64.tar.gz) this script is install.sh,
# next to the program, already built. In the source folder it is
# installer/install-linux.sh, and it builds Charpick first, with Rust.

set -eu

here=$(cd "$(dirname "$0")" && pwd)
data_home=${XDG_DATA_HOME:-$HOME/.local/share}
config_home=${XDG_CONFIG_HOME:-$HOME/.config}
bin_dir=$HOME/.local/bin
bin=$bin_dir/charpick
desktop_file=$data_home/applications/charpick.desktop
autostart_file=$config_home/autostart/charpick.desktop
settings_dir=$config_home/charpick

say() { printf '%s\n' "$*"; }
warn() { printf 'Warning: %s\n' "$*" >&2; }
die() { printf 'Error: %s\n' "$*" >&2; exit 1; }

usage() {
    sed -n '2,9s/^# \{0,1\}//p' "$0"
    exit "${1:-0}"
}

# Quote a path for the Exec key of a desktop file (see exec_quote in src/linux/mod.rs).
desktop_quote() {
    printf '"%s"' "$(printf '%s' "$1" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g' -e 's/%/%%/g')"
}

# Let Plasma's menu and shortcut settings see the new or removed desktop file.
refresh_menu() {
    if command -v kbuildsycoca6 >/dev/null 2>&1; then
        kbuildsycoca6 >/dev/null 2>&1 || true
    elif command -v kbuildsycoca5 >/dev/null 2>&1; then
        kbuildsycoca5 >/dev/null 2>&1 || true
    fi
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$data_home/applications" 2>/dev/null || true
    fi
}

install_app() {
    [ "$(uname -s)" = Linux ] || die "this script is for Linux. On Windows, run Charpick-Setup.exe."
    if [ -z "${WAYLAND_DISPLAY:-}" ]; then
        warn "this is not a Wayland session. Charpick needs KDE Plasma on Wayland and will not start under X11."
    fi
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:KDE:*) ;;
        *) warn "this is not KDE Plasma. Charpick needs KWin for the picker panel and for typing characters." ;;
    esac

    if [ -f "$here/charpick" ]; then
        # The release download: the program is already built, next to this script.
        src=$here/charpick
    else
        root=$(cd "$here/.." && pwd)
        command -v cargo >/dev/null 2>&1 || die "Rust is needed to build Charpick. Install it from https://rustup.rs and run this again."
        say "Building Charpick…"
        (cd "$root" && cargo build --release) ||
            die "the build failed. It needs the development files of cairo, pango and libxkbcommon
       (Arch: cairo pango libxkbcommon · Debian/Ubuntu: libcairo2-dev libpango1.0-dev libxkbcommon-dev
        · Fedora: cairo-devel pango-devel libxkbcommon-devel) and pkg-config."
        src=$root/target/release/Charpick
    fi

    # A reinstall keeps "Start at login" the way it was.
    if [ "$autostart" = auto ]; then
        if [ -e "$bin" ] && [ ! -e "$autostart_file" ]; then autostart=no; else autostart=yes; fi
    fi

    # Stop the running copy, whichever build it is.
    "$src" --quit || true

    mkdir -p "$bin_dir"
    install -m 755 "$src" "$bin.new"
    mv -f "$bin.new" "$bin"

    exec=$(desktop_quote "$bin")
    mkdir -p "$(dirname "$desktop_file")"
    # The file name matches Charpick's shortcut component, so System Settings shows
    # its name and icon. X-KDE-Wayland-Interfaces lets KWin hand Charpick the
    # interface it types with.
    cat >"$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Name=Charpick
GenericName=Character Picker
Comment=Accented letters and symbols
Exec=$exec
Icon=accessories-character-map
Terminal=false
Categories=Utility;
Keywords=accent;accented;letter;character;symbol;unicode;diacritic;
StartupNotify=false
X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input
Actions=change-shortcut;

[Desktop Action change-shortcut]
Name=Change Shortcut…
Exec=$exec --change-shortcut
EOF

    # The same file the tray menu's "Start at login" writes.
    if [ "$autostart" = yes ]; then
        mkdir -p "$(dirname "$autostart_file")"
        cat >"$autostart_file" <<EOF
[Desktop Entry]
Type=Application
Name=Charpick
Comment=Accented letters and symbols
Exec=$exec --autostart
Icon=accessories-character-map
Terminal=false
X-KDE-Wayland-Interfaces=org_kde_kwin_fake_input
EOF
    else
        rm -f "$autostart_file"
    fi

    refresh_menu

    say ""
    say "Charpick is installed in $bin."
    if [ "$autostart" = yes ]; then say "It starts when you log in."; fi
    case ":$PATH:" in
        *":$bin_dir:"*) ;;
        *) say "Add $bin_dir to your PATH to run \"charpick\" from a terminal." ;;
    esac

    if [ -n "${WAYLAND_DISPLAY:-}" ]; then
        if command -v setsid >/dev/null 2>&1; then
            setsid -f "$bin" </dev/null >/dev/null 2>&1
        else
            nohup "$bin" </dev/null >/dev/null 2>&1 &
        fi
        say "Charpick is running: press Meta+, (or your own shortcut) in any text field."
        say "To change the shortcut, choose \"Change shortcut…\" in its tray menu."
    fi
}

uninstall_app() {
    if [ -x "$bin" ]; then "$bin" --quit || true; fi

    # Remove the shortcut from System Settings > Keyboard > Shortcuts.
    if command -v busctl >/dev/null 2>&1; then
        busctl --user call org.kde.kglobalaccel /kglobalaccel org.kde.KGlobalAccel \
            unregister ss charpick toggle >/dev/null 2>&1 || true
    elif command -v gdbus >/dev/null 2>&1; then
        gdbus call --session --dest org.kde.kglobalaccel --object-path /kglobalaccel \
            --method org.kde.KGlobalAccel.unregister charpick toggle >/dev/null 2>&1 || true
    fi

    rm -f "$bin" "$desktop_file" "$autostart_file"
    # Records what Plasma had agreed to; Plasma has just forgotten it.
    rm -f "$settings_dir/kde-shortcut"
    refresh_menu

    if [ "$purge" = yes ]; then
        rm -rf "$settings_dir"
        say "Charpick and its settings are removed."
    else
        say "Charpick is removed. Its settings stay in $settings_dir (use --purge to remove them too)."
    fi
}

action=install
autostart=auto
purge=no
for arg in "$@"; do
    case $arg in
        --uninstall) action=uninstall ;;
        --no-autostart) autostart=no ;;
        --purge) purge=yes ;;
        -h | --help) usage ;;
        *) printf 'Unknown option: %s\n\n' "$arg" >&2; usage 2 ;;
    esac
done

[ "$(id -u)" -ne 0 ] || die "run this as yourself, without sudo: Charpick installs into your home directory."

if [ "$action" = install ]; then install_app; else uninstall_app; fi

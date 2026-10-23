#!/usr/bin/env bash
# Starts this server's Ragnarok Online client on Linux, with Wine and DXVK.
#
# Put this file in the client folder (the one with data.grf in it) and run:
#   bash play-on-linux.sh          start the launcher, which starts the game
#   bash play-on-linux.sh setup    open OpenSetup to pick DirectX 9 and a resolution
#   bash play-on-linux.sh run X    run any other Windows program in the same Wine setup
#
# The first run makes a Wine prefix just for the game, installs the core
# fonts and DXVK into it (DXVK runs the client's DirectX 9 renderer on
# Vulkan). OpenSetup opens before the game the first time a client folder
# runs in a Wine setup. Settings, all optional:
#   WINEPREFIX=...  the Wine prefix to use (default ~/.local/share/rag-wrapper-wine)
#   RO_FPS=...      the frame cap, written to ~/.config/dxvk-ro.conf on the first run
#   RO_DXVK=0       play without DXVK, on Wine's own Direct3D, for a graphics
#                   driver without Vulkan
#   RO_FONTS=0      don't install the core fonts, for a computer that can't
#                   download them
set -euo pipefail

patcher=rag-wrapper-patcher.exe
client=rag-wrapper.exe
export WINEPREFIX="${WINEPREFIX:-$HOME/.local/share/rag-wrapper-wine}"
export WINEDEBUG="${WINEDEBUG:--all}"
dxvk_conf="${XDG_CONFIG_HOME:-$HOME/.config}/dxvk-ro.conf"

die() { echo "play-on-linux: $*" >&2; exit 1; }
warn() { echo "play-on-linux: $*" >&2; }

# The client finds its GRFs from the folder it starts in. A program given
# to "run" is found from where the script was started.
if [ "${1:-}" = run ] && [ $# -ge 2 ] && [ -e "$2" ]; then
	set -- run "$(readlink -f "$2")" "${@:3}"
fi
cd "$(dirname "$(readlink -f "$0")")"
if [ "${1:-}" != run ] && [ ! -e data.grf ]; then
	die "put this script in the client folder, next to data.grf"
fi
command -v wine >/dev/null || die "Wine isn't installed; install it with your distribution's package manager"

if [ ! -d "$WINEPREFIX" ]; then
	echo "Making a Wine prefix for the game in $WINEPREFIX..."
	wineboot --init
fi

# The client's windows and text need Microsoft's core fonts, which a new
# prefix doesn't have. winetricks downloads them, so this can fail without
# internet; the game still starts, and the next run tries again.
if [ "${RO_FONTS:-1}" != 0 ] && [ ! -e "$WINEPREFIX/.rag-wrapper-corefonts" ]; then
	echo "Installing the core fonts into the prefix..."
	if ! command -v winetricks >/dev/null; then
		warn "winetricks isn't installed, so the core fonts can't be installed and text may look wrong. Install winetricks with your distribution's package manager, or run with RO_FONTS=0 to skip the fonts."
	elif winetricks -q corefonts; then
		touch "$WINEPREFIX/.rag-wrapper-corefonts"
	else
		warn "couldn't install the core fonts, so text may look wrong. Run this script again with internet to try again, or with RO_FONTS=0 to skip them."
	fi
fi

if [ "${RO_DXVK:-1}" != 0 ] && [ ! -e "$WINEPREFIX/.rag-wrapper-dxvk" ]; then
	if command -v vulkaninfo >/dev/null && ! vulkaninfo --summary >/dev/null 2>&1; then
		echo "Warning: vulkaninfo can't find a Vulkan graphics card, and DXVK needs one." >&2
		echo "If the game won't start, run with RO_DXVK=0." >&2
	fi
	command -v winetricks >/dev/null || die "winetricks isn't installed; install it with your distribution's package manager, or run with RO_DXVK=0 to play without DXVK"
	echo "Installing DXVK into the prefix..."
	winetricks -q dxvk
	touch "$WINEPREFIX/.rag-wrapper-dxvk"
fi

# DXVK reads the frame cap from this file. It lives outside the client
# folder so the patcher never sees it.
if [ ! -e "$dxvk_conf" ]; then
	mkdir -p "$(dirname "$dxvk_conf")"
	echo "d3d9.maxFrameRate = ${RO_FPS:-60}" >"$dxvk_conf"
fi
export DXVK_CONFIG_FILE="$dxvk_conf"
if [ "${RO_DXVK:-1}" = 0 ]; then
	export WINEDLLOVERRIDES="d3d9=b${WINEDLLOVERRIDES:+;$WINEDLLOVERRIDES}"
fi

# The client saves the graphics card it uses in savedata/OptionInfo.lua, in
# this folder, as the Wine setup it ran in saw it. In another prefix (even
# a new one at the same path), or with DXVK switched on or off, it runs
# without ever opening its window until OpenSetup saves the card again.
# This marker, also in this folder, names the setup OpenSetup was last
# saved in.
[ -e "$WINEPREFIX/.rag-wrapper-made" ] || date +%s >"$WINEPREFIX/.rag-wrapper-made"
dxvk=on
if [ "${RO_DXVK:-1}" = 0 ]; then
	dxvk=off
fi
setup_marker=.rag-wrapper-setup
setup_now="prefix=$WINEPREFIX made=$(cat "$WINEPREFIX/.rag-wrapper-made") dxvk=$dxvk"

case "${1:-}" in
setup)
	wine opensetup.exe
	echo "$setup_now" >"$setup_marker"
	;;
run)
	[ $# -ge 2 ] || die "say which program to run, like: bash play-on-linux.sh run WARPGATE.exe"
	shift
	exec wine "$@"
	;;
"")
	if [ "$(cat "$setup_marker" 2>/dev/null)" != "$setup_now" ]; then
		echo "Opening OpenSetup: set the renderer to DirectX 9, pick your graphics card, then save and close it."
		wine opensetup.exe
		echo "$setup_now" >"$setup_marker"
	fi
	if [ -e "$patcher" ]; then
		exec wine "$patcher"
	elif [ -e "$client" ]; then
		exec wine "$client"
	fi
	die "no $patcher in this folder; download the launcher from the server's How to connect page and extract it here"
	;;
*) die "unknown command '$1'; use setup, run or nothing" ;;
esac

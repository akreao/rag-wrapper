#!/usr/bin/env bash
# Starts this server's Ragnarok Online client on Linux, with Wine and DXVK.
#
# Put this file in the client folder (the one with data.grf in it) and run:
#   bash play-on-linux.sh          start the patcher, which starts the game
#   bash play-on-linux.sh setup    open OpenSetup to pick DirectX 9 and a resolution
#   bash play-on-linux.sh run X    run any other Windows program in the same Wine setup
#
# The first run makes a Wine prefix just for the game and installs DXVK into
# it, which runs the client's DirectX 9 renderer on Vulkan. Settings, all
# optional:
#   WINEPREFIX=...  the Wine prefix to use (default ~/.local/share/rag-wrapper-wine)
#   RO_FPS=...      the frame cap, written to ~/.config/dxvk-ro.conf on the first run
#   RO_DXVK=0       play without DXVK, on Wine's own Direct3D, for a graphics
#                   driver without Vulkan
set -euo pipefail

patcher=rag-wrapper-patcher.exe
client=rag-wrapper.exe
export WINEPREFIX="${WINEPREFIX:-$HOME/.local/share/rag-wrapper-wine}"
export WINEDEBUG="${WINEDEBUG:--all}"
dxvk_conf="${XDG_CONFIG_HOME:-$HOME/.config}/dxvk-ro.conf"

die() { echo "play-on-linux: $*" >&2; exit 1; }

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

if [ "${RO_DXVK:-1}" != 0 ] && [ ! -e "$WINEPREFIX/.rag-wrapper-dxvk" ]; then
	command -v winetricks >/dev/null || die "winetricks isn't installed; install it, or run with RO_DXVK=0 to play without DXVK"
	if command -v vulkaninfo >/dev/null && ! vulkaninfo --summary >/dev/null 2>&1; then
		echo "Warning: vulkaninfo can't find a Vulkan graphics card, and DXVK needs one." >&2
		echo "If the game won't start, run with RO_DXVK=0." >&2
	fi
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

case "${1:-}" in
setup) exec wine opensetup.exe ;;
run)
	[ $# -ge 2 ] || die "say which program to run, like: bash play-on-linux.sh run WARPGATE.exe"
	shift
	exec wine "$@"
	;;
"")
	if [ -e "$patcher" ]; then
		exec wine "$patcher"
	elif [ -e "$client" ]; then
		exec wine "$client"
	fi
	die "no $patcher in this folder; download the patcher from the server's How to connect page and extract it here"
	;;
*) die "unknown command '$1'; use setup, run or nothing" ;;
esac

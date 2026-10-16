package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"
)

// The window is plain Win32 through user32, so it needs no browser engine
// (rpatchur used Internet Explorer's) and runs under Wine as well as on
// Windows. Patching runs on its own goroutine; it posts wmRefresh, and the
// window thread copies the shared state into the controls.

var (
	user32   = windows.NewLazySystemDLL("user32.dll")
	gdi32    = windows.NewLazySystemDLL("gdi32.dll")
	comctl32 = windows.NewLazySystemDLL("comctl32.dll")
	shell32  = windows.NewLazySystemDLL("shell32.dll")

	pRegisterClassExW    = user32.NewProc("RegisterClassExW")
	pCreateWindowExW     = user32.NewProc("CreateWindowExW")
	pDefWindowProcW      = user32.NewProc("DefWindowProcW")
	pDestroyWindow       = user32.NewProc("DestroyWindow")
	pGetMessageW         = user32.NewProc("GetMessageW")
	pIsDialogMessageW    = user32.NewProc("IsDialogMessageW")
	pTranslateMessage    = user32.NewProc("TranslateMessage")
	pDispatchMessageW    = user32.NewProc("DispatchMessageW")
	pPostQuitMessage     = user32.NewProc("PostQuitMessage")
	pPostMessageW        = user32.NewProc("PostMessageW")
	pSendMessageW        = user32.NewProc("SendMessageW")
	pShowWindow          = user32.NewProc("ShowWindow")
	pEnableWindow        = user32.NewProc("EnableWindow")
	pSetWindowTextW      = user32.NewProc("SetWindowTextW")
	pMessageBoxW         = user32.NewProc("MessageBoxW")
	pLoadCursorW         = user32.NewProc("LoadCursorW")
	pLoadIconW           = user32.NewProc("LoadIconW")
	pGetSysColorBrush    = user32.NewProc("GetSysColorBrush")
	pAdjustWindowRect    = user32.NewProc("AdjustWindowRect")
	pGetSystemMetrics    = user32.NewProc("GetSystemMetrics")
	pSetProcessDPIAware  = user32.NewProc("SetProcessDPIAware")
	pGetDC               = user32.NewProc("GetDC")
	pReleaseDC           = user32.NewProc("ReleaseDC")
	pSetFocus            = user32.NewProc("SetFocus")
	pGetDeviceCaps       = gdi32.NewProc("GetDeviceCaps")
	pCreateFontW         = gdi32.NewProc("CreateFontW")
	pInitCommonControlsE = comctl32.NewProc("InitCommonControlsEx")
	pShellExecuteW       = shell32.NewProc("ShellExecuteW")
)

const (
	wsOverlapped  = 0x00000000
	wsCaption     = 0x00C00000
	wsSysMenu     = 0x00080000
	wsMinimizeBox = 0x00020000
	wsChild       = 0x40000000
	wsVisible     = 0x10000000
	wsTabStop     = 0x00010000
	wsVScroll     = 0x00200000
	wsBorder      = 0x00800000
	wsExClientEdg = 0x00000200

	esMultiline   = 0x0004
	esReadOnly    = 0x0800
	esAutoVScroll = 0x0040
	bsDefPush     = 0x0001
	ssNoPrefix    = 0x0080
	ssEndEllipsis = 0x4000

	wmDestroy     = 0x0002
	wmClose       = 0x0010
	wmSetFont     = 0x0030
	wmCommand     = 0x0111
	wmApp         = 0x8000
	wmRefresh     = wmApp + 1
	pbmSetRange32 = 0x0406
	pbmSetPos     = 0x0402

	swHide       = 0
	swShow       = 5
	cwUseDefault = 0x80000000
	colorBtnFace = 15
	idcArrow     = 32512
	idiApp       = 32512
	logPixelsY   = 90
	smCxScreen   = 0
	smCyScreen   = 1

	mbOK       = 0x00000000
	mbYesNo    = 0x00000004
	mbIconWarn = 0x00000030
	mbIconErr  = 0x00000010
	idYes      = 6

	iccProgress = 0x00000020
	iccStandard = 0x00004000
)

// Control ids.
const (
	idPlay = 100 + iota
	idRetry
	idSetup
	idExit
	idAccount
	idConnect
	idNews
)

type wndClassEx struct {
	Size       uint32
	Style      uint32
	WndProc    uintptr
	ClsExtra   int32
	WndExtra   int32
	Instance   windows.Handle
	Icon       windows.Handle
	Cursor     windows.Handle
	Background windows.Handle
	MenuName   *uint16
	ClassName  *uint16
	IconSm     windows.Handle
}

type msg struct {
	Hwnd    windows.HWND
	Message uint32
	WParam  uintptr
	LParam  uintptr
	Time    uint32
	Pt      struct{ X, Y int32 }
	Private uint32
}

type rect struct{ Left, Top, Right, Bottom int32 }

// shared is what the patching goroutine tells the window.
type shared struct {
	sync.Mutex
	title    string
	news     string
	status   string
	fraction float64
	ready    bool // patched: Play is on
	failed   bool // stopped on an error: Try again shows
	links    [3]string
	quit     bool // a new patcher started: close this one
}

type window struct {
	hwnd     windows.HWND
	dpi      int32
	controls map[int]windows.HWND
	title    windows.HWND
	news     windows.HWND
	status   windows.HWND
	bar      windows.HWND
	state    *shared
	dir      string
	exe      string
	cfg      config
	updated  bool
	shown    map[windows.HWND]string // each control's text, to leave unchanged ones alone
}

var theWindow *window

func run() int {
	runtime.LockOSThread()
	pSetProcessDPIAware.Call()
	icc := struct{ Size, ICC uint32 }{8, iccProgress | iccStandard}
	pInitCommonControlsE.Call(uintptr(unsafe.Pointer(&icc)))

	// The path Windows started the patcher from. It isn't resolved any
	// further (filepath.EvalSymlinks): under Wine that can fail with "Path
	// not found" on folders it reads differently, and the client folder is
	// simply where the patcher is.
	exe, err := os.Executable()
	if err != nil {
		messageBox(0, "Patcher", "Couldn't find the patcher's own folder: "+err.Error(), mbIconErr)
		return 1
	}
	dir := filepath.Dir(exe)
	updated := false
	for _, a := range os.Args[1:] {
		updated = updated || a == updatedFlag
	}
	if updated {
		go removeOld(exe)
	}

	cfg, err := loadConfig(dir)
	if err != nil {
		text := "The patcher's settings file, " + configName + ", is missing or damaged.\n\nDownload the patcher again from the server's \"How to connect\" page and extract everything in it into the game client's folder."
		if !errors.Is(err, os.ErrNotExist) {
			text += "\n\n(" + err.Error() + ")"
		}
		messageBox(0, "Patcher", text, mbIconErr)
		return 1
	}
	if !looksLikeClient(dir) {
		text := "This folder has no data.grf, so it doesn't look like a game client folder:\n\n" + dir + "\n\nThe patcher puts the server's files in the folder it's in. Patch this folder anyway?"
		if messageBox(0, "Patcher", text, mbYesNo|mbIconWarn) != idYes {
			return 0
		}
	}

	w := &window{dir: dir, exe: exe, cfg: cfg, updated: updated, controls: map[int]windows.HWND{}, shown: map[windows.HWND]string{},
		state: &shared{title: "Patcher", status: "Connecting to " + cfg.Portal + "..."}}
	theWindow = w
	if err := w.create(); err != nil {
		messageBox(0, "Patcher", err.Error(), mbIconErr)
		return 1
	}
	go w.patch()

	var m msg
	for {
		r, _, _ := pGetMessageW.Call(uintptr(unsafe.Pointer(&m)), 0, 0, 0)
		if int32(r) <= 0 {
			break
		}
		if r, _, _ := pIsDialogMessageW.Call(uintptr(w.hwnd), uintptr(unsafe.Pointer(&m))); r != 0 {
			continue
		}
		pTranslateMessage.Call(uintptr(unsafe.Pointer(&m)))
		pDispatchMessageW.Call(uintptr(unsafe.Pointer(&m)))
	}
	return 0
}

// patch is the patching goroutine: server info, self-update, then the
// client's patches.
func (w *window) patch() {
	s := w.state
	set := func(f func()) {
		s.Lock()
		f()
		s.Unlock()
		pPostMessageW.Call(uintptr(w.hwnd), wmRefresh, 0, 0)
	}
	set(func() {
		s.failed, s.ready, s.fraction = false, false, 0
		s.status = "Connecting to " + w.cfg.Portal + "..."
	})
	ctx := context.Background()
	info, err := fetchInfo(ctx, w.dir, &w.cfg)
	var untrusted *trustError
	if errors.As(err, &untrusted) {
		set(func() { s.failed, s.status = true, untrusted.Error() })
		return
	}
	if err != nil {
		set(func() {
			s.failed = true
			s.status = "Can't reach the server at " + w.cfg.Portal + ". Check that this PC is on the server's network and the server is running."
			s.news = "(" + err.Error() + ")"
		})
		return
	}
	set(func() {
		s.title = info.ServerName
		s.links = [3]string{safeLink(w.cfg.Portal, info.AccountURL), safeLink(w.cfg.Portal, info.ConnectURL), safeLink(w.cfg.Portal, info.NewsURL)}
		s.news = "No news yet."
		if info.News != nil {
			s.news = info.News.Title + "\n\n" + info.News.Text
		}
	})

	if !w.updated && needsUpdate(info, w.exe) {
		set(func() { s.status = "Updating the patcher..." })
		if err := replaceSelf(ctx, info, w.exe); err == nil {
			if shellExecute(w.hwnd, w.exe, updatedFlag, w.dir) == nil {
				set(func() { s.quit = true })
				return
			}
		}
		// The old patcher can still patch; it tries again next time.
	}

	err = update(ctx, w.dir, info, func(text string, fraction float64) {
		set(func() { s.status, s.fraction = text, fraction })
	})
	if err != nil {
		set(func() { s.failed, s.status = true, "Couldn't update the game: "+err.Error() })
		return
	}
	set(func() { s.ready = true })
}

func (w *window) create() error {
	inst, _, _ := windows.NewLazySystemDLL("kernel32.dll").NewProc("GetModuleHandleW").Call(0)
	className, _ := windows.UTF16PtrFromString("RagWrapperPatcher")
	cursor, _, _ := pLoadCursorW.Call(0, idcArrow)
	icon, _, _ := pLoadIconW.Call(inst, 1) // go-winres puts the icon, if any, at 1
	if icon == 0 {
		icon, _, _ = pLoadIconW.Call(0, idiApp)
	}
	bg, _, _ := pGetSysColorBrush.Call(colorBtnFace)
	wc := wndClassEx{
		WndProc:    windows.NewCallback(wndProc),
		Instance:   windows.Handle(inst),
		Icon:       windows.Handle(icon),
		Cursor:     windows.Handle(cursor),
		Background: windows.Handle(bg),
		ClassName:  className,
	}
	wc.Size = uint32(unsafe.Sizeof(wc))
	if r, _, err := pRegisterClassExW.Call(uintptr(unsafe.Pointer(&wc))); r == 0 {
		return fmt.Errorf("RegisterClassEx: %v", err)
	}

	dc, _, _ := pGetDC.Call(0)
	dpi, _, _ := pGetDeviceCaps.Call(dc, logPixelsY)
	pReleaseDC.Call(0, dc)
	w.dpi = int32(dpi)
	if w.dpi == 0 {
		w.dpi = 96
	}

	style := uintptr(wsOverlapped | wsCaption | wsSysMenu | wsMinimizeBox)
	r := rect{0, 0, w.px(560), w.px(420)}
	pAdjustWindowRect.Call(uintptr(unsafe.Pointer(&r)), style, 0)
	width, height := r.Right-r.Left, r.Bottom-r.Top
	sw, _, _ := pGetSystemMetrics.Call(smCxScreen)
	sh, _, _ := pGetSystemMetrics.Call(smCyScreen)
	title, _ := windows.UTF16PtrFromString("Patcher")
	hwnd, _, err := pCreateWindowExW.Call(0, uintptr(unsafe.Pointer(className)), uintptr(unsafe.Pointer(title)), style,
		uintptr((int32(sw)-width)/2), uintptr((int32(sh)-height)/2), uintptr(width), uintptr(height), 0, 0, inst, 0)
	if hwnd == 0 {
		return fmt.Errorf("CreateWindowEx: %v", err)
	}
	w.hwnd = windows.HWND(hwnd)

	font := w.font(15, 400)
	bold := w.font(22, 700)
	w.title = w.control("STATIC", "", ssNoPrefix|ssEndEllipsis, 0, 16, 12, 528, 32, 0, bold)
	w.news = w.control("EDIT", "", esMultiline|esReadOnly|esAutoVScroll|wsVScroll|wsTabStop, wsExClientEdg, 16, 52, 528, 200, 0, font)
	w.status = w.control("STATIC", "", ssNoPrefix, 0, 16, 262, 528, 36, 0, font)
	w.bar = w.control("msctls_progress32", "", 0, 0, 16, 300, 528, 18, 0, 0)
	pSendMessageW.Call(uintptr(w.bar), pbmSetRange32, 0, 1000)
	w.control("BUTTON", "Play", bsDefPush|wsTabStop, 0, 16, 334, 130, 32, idPlay, font)
	w.control("BUTTON", "Try again", wsTabStop, 0, 154, 334, 110, 32, idRetry, font)
	w.control("BUTTON", "Settings", wsTabStop, 0, 326, 334, 100, 32, idSetup, font)
	w.control("BUTTON", "Exit", wsTabStop, 0, 434, 334, 110, 32, idExit, font)
	w.control("BUTTON", "Your account", wsTabStop, 0, 16, 380, 130, 26, idAccount, font)
	w.control("BUTTON", "How to connect", wsTabStop, 0, 154, 380, 130, 26, idConnect, font)
	w.control("BUTTON", "News", wsTabStop, 0, 292, 380, 100, 26, idNews, font)

	w.refresh()
	pShowWindow.Call(hwnd, swShow)
	return nil
}

// px scales a size at 96 DPI to the screen's.
func (w *window) px(n int32) int32 { return n * w.dpi / 96 }

func (w *window) font(size, weight int32) uintptr {
	face, _ := windows.UTF16PtrFromString("Segoe UI")
	const defaultCharset, clearTypeQuality = 1, 5
	f, _, _ := pCreateFontW.Call(uintptr(-w.px(size)), 0, 0, 0, uintptr(weight), 0, 0, 0,
		defaultCharset, 0, 0, clearTypeQuality, 0, uintptr(unsafe.Pointer(face)))
	return f
}

func (w *window) control(class, text string, style, exStyle uintptr, x, y, cx, cy int32, id int, font uintptr) windows.HWND {
	c, _ := windows.UTF16PtrFromString(class)
	t, _ := windows.UTF16PtrFromString(text)
	h, _, _ := pCreateWindowExW.Call(exStyle, uintptr(unsafe.Pointer(c)), uintptr(unsafe.Pointer(t)),
		wsChild|wsVisible|style, uintptr(w.px(x)), uintptr(w.px(y)), uintptr(w.px(cx)), uintptr(w.px(cy)),
		uintptr(w.hwnd), uintptr(id), 0, 0)
	if font != 0 {
		pSendMessageW.Call(h, wmSetFont, font, 1)
	}
	if id != 0 {
		w.controls[id] = windows.HWND(h)
	}
	return windows.HWND(h)
}

// refresh copies the shared state into the controls.
func (w *window) refresh() {
	s := w.state
	s.Lock()
	defer s.Unlock()
	if s.quit {
		pDestroyWindow.Call(uintptr(w.hwnd))
		return
	}
	w.setText(w.hwnd, s.title+" patcher")
	w.setText(w.title, s.title)
	w.setText(w.news, strings.ReplaceAll(strings.ReplaceAll(s.news, "\r\n", "\n"), "\n", "\r\n"))
	w.setText(w.status, s.status)
	pSendMessageW.Call(uintptr(w.bar), pbmSetPos, uintptr(int(s.fraction*1000)), 0)
	enable(w.controls[idPlay], s.ready)
	show(w.controls[idRetry], s.failed)
	for i, id := range []int{idAccount, idConnect, idNews} {
		enable(w.controls[id], s.links[i] != "")
	}
	if s.ready {
		pSetFocus.Call(uintptr(w.controls[idPlay]))
	}
}

func wndProc(hwnd, message, wParam, lParam uintptr) uintptr {
	w := theWindow
	switch message {
	case wmRefresh:
		w.refresh()
		return 0
	case wmCommand:
		w.command(int(wParam & 0xffff))
		return 0
	case wmClose:
		pDestroyWindow.Call(hwnd)
		return 0
	case wmDestroy:
		pPostQuitMessage.Call(0)
		return 0
	}
	r, _, _ := pDefWindowProcW.Call(hwnd, message, wParam, lParam)
	return r
}

func (w *window) command(id int) {
	s := w.state
	s.Lock()
	links, ready, failed := s.links, s.ready, s.failed
	s.Unlock()
	switch id {
	case idPlay:
		if !ready {
			return
		}
		if err := w.start(clientExe, "The game, "+clientExe+", isn't in this folder. Click Try again to download it, or ask the server's owner whether they've published the client."); err == nil {
			pDestroyWindow.Call(uintptr(w.hwnd))
		}
	case idSetup:
		w.start(setupExe, setupExe+", the client's settings program, isn't in this folder. It comes with the client from WARPGATE.")
	case idRetry:
		if failed {
			s.Lock()
			s.failed = false
			s.Unlock()
			show(w.controls[idRetry], false)
			go w.patch()
		}
	case idExit:
		pDestroyWindow.Call(uintptr(w.hwnd))
	case idAccount, idConnect, idNews:
		if u := links[id-idAccount]; u != "" {
			shellExecute(w.hwnd, u, "", "")
		}
	}
}

// start runs a program from the client folder.
func (w *window) start(name, missing string) error {
	path := filepath.Join(w.dir, name)
	if _, err := os.Stat(path); err != nil {
		messageBox(w.hwnd, "Patcher", missing, mbIconWarn)
		return err
	}
	if err := shellExecute(w.hwnd, path, "", w.dir); err != nil {
		messageBox(w.hwnd, "Patcher", "Couldn't start "+name+": "+err.Error(), mbIconErr)
		return err
	}
	return nil
}

// shellExecute opens a program or link the way Explorer does, so a client
// that asks for administrator rights gets Windows' prompt.
func shellExecute(hwnd windows.HWND, file, args, dir string) error {
	verb, _ := windows.UTF16PtrFromString("open")
	f, _ := windows.UTF16PtrFromString(file)
	var a, d *uint16
	if args != "" {
		a, _ = windows.UTF16PtrFromString(args)
	}
	if dir != "" {
		d, _ = windows.UTF16PtrFromString(dir)
	}
	r, _, _ := pShellExecuteW.Call(uintptr(hwnd), uintptr(unsafe.Pointer(verb)), uintptr(unsafe.Pointer(f)),
		uintptr(unsafe.Pointer(a)), uintptr(unsafe.Pointer(d)), swShow)
	if r <= 32 {
		return fmt.Errorf("error %d", r)
	}
	return nil
}

func messageBox(hwnd windows.HWND, title, text string, flags uintptr) int {
	t, _ := windows.UTF16PtrFromString(title)
	m, _ := windows.UTF16PtrFromString(text)
	r, _, _ := pMessageBoxW.Call(uintptr(hwnd), uintptr(unsafe.Pointer(m)), uintptr(unsafe.Pointer(t)), flags|mbOK)
	return int(r)
}

// setText changes a control's text if it differs, so the news box keeps
// its scroll position while the status changes.
func (w *window) setText(h windows.HWND, s string) {
	if w.shown[h] == s {
		return
	}
	w.shown[h] = s
	p, _ := syscall.UTF16PtrFromString(strings.ReplaceAll(s, "\x00", ""))
	pSetWindowTextW.Call(uintptr(h), uintptr(unsafe.Pointer(p)))
}

func enable(h windows.HWND, on bool) {
	v := uintptr(0)
	if on {
		v = 1
	}
	pEnableWindow.Call(uintptr(h), v)
}

func show(h windows.HWND, on bool) {
	v := uintptr(swHide)
	if on {
		v = swShow
	}
	pShowWindow.Call(uintptr(h), v)
}

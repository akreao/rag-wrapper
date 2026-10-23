package main

import (
	"context"
	"errors"
	"fmt"
	"image"
	"image/color"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"
	"unsafe"

	"gioui.org/app"
	"gioui.org/font"
	"gioui.org/io/pointer"
	"gioui.org/io/system"
	"gioui.org/layout"
	"gioui.org/op"
	"gioui.org/op/paint"
	"gioui.org/text"
	"gioui.org/unit"
	"gioui.org/widget"
	"gioui.org/widget/material"
	"golang.org/x/sys/windows"
)

// The launcher's window. Gio draws it (no browser engine: rpatchur used
// Internet Explorer's), with Direct3D 11 or OpenGL, so it runs under Wine
// as well as on Windows. Gio stays at v0.8.0: later versions read the
// mouse only through WM_POINTER messages, which Wine 9 doesn't send, and
// fail to open a window there.
//
// Patching and the server's live info run on their own goroutines. They
// change the shared status under its lock and ask for a new frame; each
// frame draws from a copy.

var (
	user32                = windows.NewLazySystemDLL("user32.dll")
	shell32               = windows.NewLazySystemDLL("shell32.dll")
	shcore                = windows.NewLazySystemDLL("shcore.dll")
	gdi32                 = windows.NewLazySystemDLL("gdi32.dll")
	pMessageBoxW          = user32.NewProc("MessageBoxW")
	pPostMessageW         = user32.NewProc("PostMessageW")
	pSetProcessDPIAware   = user32.NewProc("SetProcessDPIAware")
	pSystemParametersInfo = user32.NewProc("SystemParametersInfoW")
	pAdjustWindowRectEx   = user32.NewProc("AdjustWindowRectEx")
	pGetDesktopWindow     = user32.NewProc("GetDesktopWindow")
	pMonitorFromWindow    = user32.NewProc("MonitorFromWindow")
	pGetMonitorInfoW      = user32.NewProc("GetMonitorInfoW")
	pGetWindowRect        = user32.NewProc("GetWindowRect")
	pSetWindowPos         = user32.NewProc("SetWindowPos")
	pGetDC                = user32.NewProc("GetDC")
	pReleaseDC            = user32.NewProc("ReleaseDC")
	pGetDeviceCaps        = gdi32.NewProc("GetDeviceCaps")
	pGetDpiForMonitor     = shcore.NewProc("GetDpiForMonitor")
	pShellExecute         = shell32.NewProc("ShellExecuteW")
	pD3D11CreateDevice    = windows.NewLazySystemDLL("d3d11.dll").NewProc("D3D11CreateDevice")
)

const (
	mbOK       = 0x00000000
	mbYesNo    = 0x00000004
	mbIconWarn = 0x00000030
	mbIconErr  = 0x00000010
	idYes      = 6
	swShow     = 5
)

// liveEvery is how often the launcher asks the server who's online.
const liveEvery = 30 * time.Second

// The window's size and smallest size, in dp, when the screen has room;
// windowSize shrinks them to fit. Below contentMin the Play and Settings
// tabs scroll, so the status line and Play always show (News and Players
// are lists that scroll anyway).
const (
	windowW, windowH = 980, 660
	minW, minH       = 860, 440
	contentMin       = 440
)

// The tabs, in order, with the names settings.Tab keeps.
type tab int

const (
	tabPlay tab = iota
	tabNews
	tabPlayers
	tabSettings
)

var tabNames = [...]string{"Play", "News", "Players", "Settings"}
var tabKeys = [...]string{"play", "news", "players", "settings"}

// status is what the goroutines tell the window.
type status struct {
	title    string
	text     string // what the patcher is doing
	detail   string // why it stopped
	fraction float64
	busy     bool // patching: Play, Try again and Check files wait
	ready    bool // patched: Play is on
	failed   bool // stopped on an error: Try again shows
	links    map[string]string
	live     *liveInfo
	up       int
	quit     bool // a new launcher started: close this one
}

type launcher struct {
	win     *app.Window
	th      *theme
	dir     string
	exe     string
	cfg     config
	updated bool

	mu sync.Mutex
	st status
	// wake asks watchLive to look again now.
	wake chan struct{}
	// hwnd is the window's, once it's open, for closing it from the patch
	// goroutine.
	hwnd atomic.Uintptr
	// fonts brings the shaper with the system fonts (systemShaper).
	fonts chan *text.Shaper

	// Only the window's goroutine uses the rest.
	set      settings
	tab      tab
	dark     bool
	notice   string // the last thing a button couldn't do
	shown    string // the window title set
	tabs     [len(tabNames)]widget.Clickable
	play     widget.Clickable
	retry    widget.Clickable
	setup    widget.Clickable
	repair   widget.Clickable
	repairOK widget.Clickable
	repairNo widget.Clickable
	confirm  bool // asking before Check game files
	folder   widget.Clickable
	shots    widget.Clickable
	links    map[string]*widget.Clickable
	keepOpen widget.Bool
	themes   widget.Enum
	lists    struct{ page, playNews, news, online, top widget.List }
}

func run() int {
	// The path Windows started the launcher from. It isn't resolved any
	// further (filepath.EvalSymlinks): under Wine that can fail with "Path
	// not found" on folders it reads differently, and the client folder is
	// simply where the launcher is.
	exe, err := os.Executable()
	if err != nil {
		messageBox("Couldn't find the launcher's own folder: "+err.Error(), mbIconErr)
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
		text := "The launcher's settings file, " + configName + ", is missing or damaged.\n\nDownload the launcher again from the server's \"How to connect\" page and extract everything in it into the game client's folder."
		if !errors.Is(err, os.ErrNotExist) {
			text += "\n\n(" + err.Error() + ")"
		}
		messageBox(text, mbIconErr)
		return 1
	}
	if !looksLikeClient(dir) {
		text := "This folder has no data.grf, so it doesn't look like a game client folder:\n\n" + dir + "\n\nThe launcher puts the server's files in the folder it's in. Patch this folder anyway?"
		if messageBox(text, mbYesNo|mbIconWarn) != idYes {
			return 0
		}
	}

	l := &launcher{
		dir: dir, exe: exe, cfg: cfg, updated: updated,
		th:    newTheme(),
		set:   loadSettings(dir),
		links: map[string]*widget.Clickable{},
		wake:  make(chan struct{}, 1),
		fonts: make(chan *text.Shaper, 1),
		st:    status{text: "Connecting to " + cfg.Portal + "...", busy: true},
	}
	for i, k := range tabKeys {
		if k == l.set.Tab {
			l.tab = tab(i)
		}
	}
	// The name from last time, until the server answers.
	l.st.title = l.set.ServerName
	l.keepOpen.Value = l.set.KeepOpen
	l.themes.Value = l.set.Theme
	l.applyTheme()
	for _, li := range []*widget.List{&l.lists.page, &l.lists.playNews, &l.lists.news, &l.lists.online, &l.lists.top} {
		li.Axis = layout.Vertical
	}

	if err := checkD3D11(); err != nil {
		noWindow(err)
		return 1
	}
	l.win = new(app.Window)
	size, least := windowSize()
	l.win.Option(app.Title("Launcher"), app.Size(size.w, size.h), app.MinSize(least.w, least.h))
	go l.loop()
	go l.patch()
	go l.watchLive()
	go func() {
		l.fonts <- systemShaper()
		l.win.Invalidate()
	}()
	app.Main()
	return 0
}

// update changes the shared status and draws a new frame.
func (l *launcher) update(f func(s *status)) {
	l.mu.Lock()
	f(&l.st)
	l.mu.Unlock()
	l.win.Invalidate()
}

func (l *launcher) snapshot() status {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.st
}

// startPatch marks the launcher busy at once, so a second click can't
// start a second update, and starts one.
func (l *launcher) startPatch() {
	l.update(func(s *status) { s.busy, s.failed, s.ready = true, false, false })
	go l.patch()
}

// patch is the patching goroutine: server info, self-update, then the
// client's patches.
func (l *launcher) patch() {
	l.update(func(s *status) {
		s.failed, s.ready, s.busy, s.fraction, s.detail = false, false, true, 0, ""
		s.text = "Connecting to " + l.cfg.Portal + "..."
	})
	ctx := context.Background()
	info, err := fetchInfo(ctx, l.dir, &l.cfg)
	var untrusted *trustError
	if errors.As(err, &untrusted) {
		l.update(func(s *status) { s.failed, s.busy, s.text = true, false, untrusted.Error() })
		return
	}
	if err != nil {
		l.update(func(s *status) {
			s.failed, s.busy = true, false
			s.text = "Can't reach the server at " + l.cfg.Portal + ". Check that this PC is on the server's network and the server is running."
			s.detail = err.Error()
		})
		return
	}
	l.update(func(s *status) {
		s.title = info.ServerName
		s.links = map[string]string{
			"account": safeLink(l.cfg.Portal, info.AccountURL),
			"connect": safeLink(l.cfg.Portal, info.ConnectURL),
			"news":    safeLink(l.cfg.Portal, info.NewsURL),
		}
	})

	if !l.updated && needsUpdate(info, l.exe) {
		l.update(func(s *status) { s.text = "Updating the launcher..." })
		if err := replaceSelf(ctx, info, l.exe); err == nil {
			if shellExecute(l.exe, updatedFlag, l.dir) == nil {
				// Closed from here, not on the next frame: a minimized
				// window draws none.
				l.update(func(s *status) { s.quit = true })
				l.close()
				return
			}
		}
		// The old launcher can still patch; it tries again next time.
	}

	err = update(ctx, l.dir, info, func(text string, fraction float64) {
		l.update(func(s *status) { s.text, s.fraction = text, fraction })
	})
	if err != nil {
		l.update(func(s *status) { s.failed, s.busy, s.text = true, false, "Couldn't update the game: "+err.Error() })
		return
	}
	l.update(func(s *status) { s.ready, s.busy = true, false })
}

// watchLive asks the server who's online and the news, and checks the
// game's login server, now and every liveEvery.
func (l *launcher) watchLive() {
	var last *liveInfo
	for {
		ctx := context.Background()
		live, err := fetchLive(ctx, l.cfg.Portal)
		if err != nil {
			live = nil
		}
		up := gameState(ctx, live, last)
		if live != nil {
			last = live
		}
		l.update(func(s *status) {
			if live != nil {
				s.live = live
			}
			s.up = up
		})
		select {
		case <-time.After(liveEvery):
		case <-l.wake:
		}
	}
}

func (l *launcher) applyTheme() {
	l.dark = l.set.Theme == themeDark || (l.set.Theme == themeSystem && windowsDark())
	if l.dark {
		l.th.use(darkPalette)
	} else {
		l.th.use(lightPalette)
	}
}

func (l *launcher) saveSettings() {
	l.set.Tab = tabKeys[l.tab]
	saveSettings(l.dir, l.set)
}

// noWindow tells the player the window couldn't open or draw, and what to
// do instead. Gio draws with Direct3D 11 only here (its OpenGL renderer
// needs ANGLE's libEGL.dll, which the download doesn't have), so the cause
// is nearly always a missing graphics driver, a very old graphics card or
// a virtual machine without 3D.
func noWindow(err error) {
	messageBox("The launcher couldn't draw its window on this PC. It needs Direct3D 11 graphics, which a missing graphics driver, a very old graphics card or a virtual machine without 3D doesn't have.\n\nOn the server's \"How to connect\" page, click \"download the old patcher\", extract it into the game client's folder the same way, and tell the server's owner.\n\n("+err.Error()+")", mbIconErr)
}

// checkD3D11 makes the Direct3D 11 device Gio will ask for, and lets it
// go. Gio's own failure isn't a clean error: when Direct3D 11 fails it
// tries OpenGL, and the second time round (the window draws again before
// it closes) its OpenGL loader crashes on the missing libEGL.dll, so the
// launcher would vanish without a word.
func checkD3D11() error {
	if err := pD3D11CreateDevice.Find(); err != nil {
		return fmt.Errorf("Direct3D 11 isn't installed: %w", err)
	}
	const driverHardware, sdkVersion, featureLevel91 = 1, 7, 0x9100
	var dev, ctx unsafe.Pointer
	var level uint32
	r, _, _ := pD3D11CreateDevice.Call(0, driverHardware, 0, 0, 0, 0, sdkVersion,
		uintptr(unsafe.Pointer(&dev)), uintptr(unsafe.Pointer(&level)), uintptr(unsafe.Pointer(&ctx)))
	for _, obj := range []unsafe.Pointer{ctx, dev} {
		if obj != nil {
			release(obj)
		}
	}
	if r != 0 {
		return fmt.Errorf("D3D11CreateDevice: error %#x", uint32(r))
	}
	if level < featureLevel91 {
		return fmt.Errorf("the graphics card's Direct3D feature level is too low (%#x)", level)
	}
	return nil
}

// release calls a COM object's Release, the third method in its table.
func release(obj unsafe.Pointer) {
	vtbl := *(**[3]uintptr)(obj)
	syscall.SyscallN(vtbl[2], uintptr(obj))
}

// close closes the window from any goroutine, minimized or not: Windows
// queues the WM_CLOSE, and Gio's DestroyEvent then ends the launcher.
func (l *launcher) close() {
	if h := l.hwnd.Load(); h != 0 {
		pPostMessageW.Call(h, wmClose, 0, 0)
	}
}

func (l *launcher) loop() {
	var ops op.Ops
	for {
		switch e := l.win.Event().(type) {
		case app.DestroyEvent:
			l.saveSettings()
			if e.Err != nil {
				noWindow(e.Err)
				os.Exit(1)
			}
			os.Exit(0)
		case app.Win32ViewEvent:
			if e.Valid() {
				l.hwnd.Store(e.HWND)
				l.win.Run(func() { fitWindow(e.HWND) })
			}
		case app.FrameEvent:
			select {
			case sh := <-l.fonts:
				l.th.Shaper = sh
			default:
			}
			gtx := app.NewContext(&ops, e)
			s := l.snapshot()
			if s.quit {
				l.win.Perform(system.ActionClose)
			}
			l.handle(gtx, s)
			l.layout(gtx, s)
			e.Frame(gtx.Ops)
		}
	}
}

// link is the address behind one of the portal's pages, from the live
// info or else the patcher info.
func (s status) link(key string) string {
	if s.live != nil && s.live.Links[key] != "" {
		return s.live.Links[key]
	}
	return s.links[key]
}

// handle acts on the clicks since the last frame.
func (l *launcher) handle(gtx layout.Context, s status) {
	for i := range l.tabs {
		if l.tabs[i].Clicked(gtx) {
			l.tab = tab(i)
			l.confirm = false
		}
	}
	if l.play.Clicked(gtx) && s.ready {
		if l.start(clientExe, "The game, "+clientExe+", isn't in this folder. Click Check game files on the Settings tab to download it, or ask the server's owner whether they've published the client.") && !l.set.KeepOpen {
			l.win.Perform(system.ActionClose)
		}
	}
	if l.retry.Clicked(gtx) && s.failed {
		l.notice = ""
		l.startPatch()
		select {
		case l.wake <- struct{}{}:
		default:
		}
	}
	if l.setup.Clicked(gtx) {
		l.start(setupExe, setupExe+", the client's settings program, isn't in this folder. It comes with the client from WARPGATE.")
	}
	if l.repair.Clicked(gtx) && !s.busy {
		l.confirm = true
	}
	if l.repairNo.Clicked(gtx) {
		l.confirm = false
	}
	if l.repairOK.Clicked(gtx) && !s.busy {
		l.confirm = false
		if err := startRepair(l.dir); err != nil {
			l.notice = "Couldn't check the game files: " + err.Error()
		} else {
			l.notice = ""
			l.startPatch()
		}
	}
	if l.folder.Clicked(gtx) {
		l.open(l.dir)
	}
	if l.shots.Clicked(gtx) {
		l.open(filepath.Join(l.dir, "ScreenShot"))
	}
	for key, c := range l.links {
		if c.Clicked(gtx) {
			if u := s.link(key); u != "" {
				l.open(u)
			}
		}
	}
	if l.keepOpen.Update(gtx) {
		l.set.KeepOpen = l.keepOpen.Value
		l.saveSettings()
	}
	if l.themes.Update(gtx) {
		l.set.Theme = l.themes.Value
		l.applyTheme()
		l.saveSettings()
	}
	if s.title != "" && s.title != l.set.ServerName {
		l.set.ServerName = s.title
		l.saveSettings()
	}
	title := "Launcher"
	if s.title != "" {
		title = s.title + " launcher"
	}
	if title != l.shown {
		l.shown = title
		l.win.Option(app.Title(title))
	}
}

// start runs a program from the client folder; false when it couldn't.
func (l *launcher) start(name, missing string) bool {
	path := filepath.Join(l.dir, name)
	if _, err := os.Stat(path); err != nil {
		l.notice = missing
		return false
	}
	if err := shellExecute(path, "", l.dir); err != nil {
		l.notice = "Couldn't start " + name + ": " + err.Error()
		return false
	}
	l.notice = ""
	return true
}

// open shows a folder in Explorer or a page in the browser.
func (l *launcher) open(target string) {
	if err := shellExecute(target, "", ""); err != nil {
		l.notice = "Couldn't open " + target + ": " + err.Error()
	}
}

func (l *launcher) linkButton(key string) *widget.Clickable {
	c := l.links[key]
	if c == nil {
		c = new(widget.Clickable)
		l.links[key] = c
	}
	return c
}

// --- Layout ---

func (l *launcher) layout(gtx layout.Context, s status) {
	t := l.th
	paint.Fill(gtx.Ops, t.p.sky)
	layout.UniformInset(16).Layout(gtx, func(gtx layout.Context) layout.Dimensions {
		return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
			layout.Rigid(func(gtx layout.Context) layout.Dimensions { return l.topBar(gtx, s) }),
			layout.Rigid(layout.Spacer{Height: 16}.Layout),
			layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
				// On a short screen the tab keeps its smallest height and
				// scrolls, rather than squashing into the bars around it.
				least := gtx.Dp(contentMin)
				if l.tab == tabNews || l.tab == tabPlayers || gtx.Constraints.Max.Y >= least {
					return l.tabContent(gtx, s)
				}
				return material.List(l.th.Theme, &l.lists.page).Layout(gtx, 1, func(gtx layout.Context, _ int) layout.Dimensions {
					gtx.Constraints.Min.Y, gtx.Constraints.Max.Y = least, least
					return layout.Inset{Right: 12}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
						return l.tabContent(gtx, s)
					})
				})
			}),
			layout.Rigid(layout.Spacer{Height: 16}.Layout),
			layout.Rigid(func(gtx layout.Context) layout.Dimensions { return l.bottomBar(gtx, s) }),
		)
	})
}

func (l *launcher) tabContent(gtx layout.Context, s status) layout.Dimensions {
	switch l.tab {
	case tabNews:
		return l.newsTab(gtx, s)
	case tabPlayers:
		return l.playersTab(gtx, s)
	case tabSettings:
		return l.settingsTab(gtx, s)
	}
	return l.playTab(gtx, s)
}

// raised lays w out in a plain raised box (no title bar).
func (l *launcher) raised(gtx layout.Context, in layout.Inset, w layout.Widget) layout.Dimensions {
	return layout.Stack{}.Layout(gtx,
		layout.Expanded(func(gtx layout.Context) layout.Dimensions {
			l.th.box(gtx, gtx.Constraints.Min, l.th.p.panel)
			return layout.Dimensions{Size: gtx.Constraints.Min}
		}),
		layout.Stacked(func(gtx layout.Context) layout.Dimensions {
			gtx.Constraints.Min.X = gtx.Constraints.Max.X
			in.Bottom += 3 // the shadow
			return in.Layout(gtx, w)
		}),
	)
}

func (l *launcher) topBar(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	return l.raised(gtx, layout.Inset{Top: 8, Bottom: 8, Left: 12, Right: 12}, func(gtx layout.Context) layout.Dimensions {
		children := []layout.FlexChild{
			layout.Rigid(func(gtx layout.Context) layout.Dimensions { return t.poring(gtx, 26) }),
			layout.Rigid(layout.Spacer{Width: 8}.Layout),
			layout.Rigid(func(gtx layout.Context) layout.Dimensions {
				gtx.Constraints.Max.X = min(gtx.Constraints.Max.X, gtx.Dp(260))
				name := s.title
				if name == "" {
					name = "Launcher"
				}
				lb := t.text(19, font.ExtraBold, t.p.fg, name)
				lb.MaxLines = 1
				return lb.Layout(gtx)
			}),
			layout.Rigid(layout.Spacer{Width: 20}.Layout),
		}
		for i := range tabNames {
			i := i
			children = append(children, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
				return l.navItem(gtx, &l.tabs[i], tabNames[i], l.tab == tab(i))
			}), layout.Rigid(layout.Spacer{Width: 2}.Layout))
		}
		children = append(children, layout.Flexed(1, layout.Spacer{}.Layout))
		if s.link("account") != "" {
			children = append(children, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
				return t.button(gtx, l.linkButton("account"), "Your account", buttonStyle{kind: btnBlue, size: 14})
			}))
		}
		return layout.Flex{Alignment: layout.Middle}.Layout(gtx, children...)
	})
}

// navItem is a tab in the top bar, like the portal's navigation links.
func (l *launcher) navItem(gtx layout.Context, c *widget.Clickable, label string, active bool) layout.Dimensions {
	t := l.th
	return c.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
		pointer.CursorPointer.Add(gtx.Ops)
		fg := t.p.fg
		macro := op.Record(gtx.Ops)
		lbl := t.text(15, font.ExtraBold, fg, label)
		if active {
			lbl.Color = rgb(0xffffff)
		}
		dims := layout.Inset{Top: 7, Bottom: 7, Left: 14, Right: 14}.Layout(gtx, lbl.Layout)
		call := macro.Stop()
		switch {
		case active:
			fillRRect(gtx, image.Rectangle{Max: dims.Size}, gtx.Dp(8), t.p.title)
		case c.Hovered():
			fillRRect(gtx, image.Rectangle{Max: dims.Size}, gtx.Dp(8), t.p.inset)
		}
		call.Add(gtx.Ops)
		return dims
	})
}

func (l *launcher) bottomBar(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	return l.raised(gtx, layout.Inset{Top: 14, Bottom: 14, Left: 18, Right: 14}, func(gtx layout.Context) layout.Dimensions {
		return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
			layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
				return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
					layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						c := t.p.fg
						if s.failed {
							c = t.p.badFg
						}
						lb := t.text(15, font.ExtraBold, c, s.text)
						lb.MaxLines = 2
						return lb.Layout(gtx)
					}),
					layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						line := s.detail
						if l.notice != "" {
							line = l.notice
						}
						if line == "" {
							return layout.Dimensions{}
						}
						lb := t.text(13.5, font.Normal, t.p.badFg, line)
						lb.MaxLines = 2
						return layout.Inset{Top: 2}.Layout(gtx, lb.Layout)
					}),
					layout.Rigid(layout.Spacer{Height: 10}.Layout),
					layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						return t.progress(gtx, float32(s.fraction), s.ready)
					}),
				)
			}),
			layout.Rigid(layout.Spacer{Width: 18}.Layout),
			layout.Rigid(func(gtx layout.Context) layout.Dimensions {
				if !s.failed {
					return layout.Dimensions{}
				}
				return layout.Inset{Right: 10}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
					return t.button(gtx, &l.retry, "Try again", buttonStyle{kind: btnPlain})
				})
			}),
			layout.Rigid(func(gtx layout.Context) layout.Dimensions {
				return t.button(gtx, &l.play, "Play", buttonStyle{kind: btnCTA, size: 20, tall: true, disabled: !s.ready})
			}),
		)
	})
}

// --- Play tab ---

func (l *launcher) playTab(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
		layout.Rigid(func(gtx layout.Context) layout.Dimensions {
			return t.panel(gtx, "Welcome", false, func(gtx layout.Context) layout.Dimensions {
				return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
					layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
						return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
							layout.Rigid(func(gtx layout.Context) layout.Dimensions {
								heading := "Welcome"
								if s.title != "" {
									heading += " to " + s.title
								}
								lb := t.text(27, font.ExtraBold, t.p.fg, heading)
								lb.MaxLines = 2
								return lb.Layout(gtx)
							}),
							layout.Rigid(layout.Spacer{Height: 6}.Layout),
							layout.Rigid(t.text(15, font.Normal, t.p.muted, welcomeText(s)).Layout),
						)
					}),
					layout.Rigid(layout.Spacer{Width: 20}.Layout),
					layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						gtx.Constraints.Max.X = gtx.Dp(340)
						gtx.Constraints.Min.X = gtx.Constraints.Max.X
						return l.tiles(gtx, s)
					}),
				)
			})
		}),
		layout.Rigid(layout.Spacer{Height: 16}.Layout),
		layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return layout.Flex{}.Layout(gtx,
				layout.Flexed(3, func(gtx layout.Context) layout.Dimensions {
					var posts []newsPost
					if s.live != nil {
						posts = s.live.News
					}
					if len(posts) > 1 {
						posts = posts[:1]
					}
					return t.panel(gtx, "Latest news", true, func(gtx layout.Context) layout.Dimensions {
						return l.posts(gtx, &l.lists.playNews, posts, s)
					})
				}),
				layout.Rigid(layout.Spacer{Width: 16}.Layout),
				layout.Flexed(2, func(gtx layout.Context) layout.Dimensions {
					return t.panel(gtx, "On the website", true, func(gtx layout.Context) layout.Dimensions {
						return l.webLinks(gtx, s)
					})
				}),
			)
		}),
	)
}

func welcomeText(s status) string {
	switch {
	case s.ready && s.up == upNo:
		return "Your game is up to date, but the game server isn't answering. It may be starting or stopped."
	case s.ready && s.up == upNoPortal:
		return "Your game is up to date. The server's website isn't answering just now, so the launcher can't tell whether the game server is up. Click Play to try."
	case s.ready:
		return "Your game is up to date. Click Play to start it."
	case s.failed:
		return "The update stopped. The reason is at the bottom of this window."
	}
	return "Getting your game up to date. Play lights up when it's ready."
}

// tiles is the server at a glance, two by two.
func (l *launcher) tiles(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	server, serverColor := "Checking...", t.p.fg
	switch s.up {
	case upYes:
		server, serverColor = "Online", t.p.okFg
	case upNo:
		server, serverColor = "Offline", t.p.badFg
	case upNoPortal:
		server, serverColor = "Unknown", t.p.muted
	}
	playing, mode, date := "-", "-", "-"
	if s.live != nil {
		if s.live.Players {
			playing = strconv.Itoa(s.live.OnlineCount)
		}
		mode, date = s.live.Mode, s.live.ClientDate
	}
	cell := func(label, value string, c color.NRGBA) layout.FlexChild {
		return layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return t.tile(gtx, label, value, c)
		})
	}
	row := func(a, b layout.FlexChild) layout.Widget {
		return func(gtx layout.Context) layout.Dimensions {
			return layout.Flex{}.Layout(gtx, a, layout.Rigid(layout.Spacer{Width: 10}.Layout), b)
		}
	}
	return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
		layout.Rigid(row(cell("Game server", server, serverColor), cell("Playing now", playing, t.p.fg))),
		layout.Rigid(layout.Spacer{Height: 10}.Layout),
		layout.Rigid(row(cell("Mode", mode, t.p.fg), cell("Client date", date, t.p.fg))),
	)
}

// webLinks are buttons for the portal's pages.
func (l *launcher) webLinks(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	items := []struct{ key, label string }{
		{"account", "Your account"},
		{"connect", "How to connect"},
		{"rankings", "Rankings"},
		{"market", "Market"},
		{"items", "Database"},
		{"news", "News and rules"},
	}
	var shown []struct{ key, label string }
	for _, it := range items {
		if s.link(it.key) != "" {
			shown = append(shown, it)
		}
	}
	if len(shown) == 0 {
		return t.muted("The server's website links show once the launcher reaches the server.").Layout(gtx)
	}
	// Two buttons a row.
	var rows []layout.FlexChild
	for i := 0; i < len(shown); i += 2 {
		pair := shown[i:min(i+2, len(shown))]
		rows = append(rows, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
			return layout.Inset{Bottom: 10}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
				cells := []layout.FlexChild{}
				for j, it := range pair {
					if j > 0 {
						cells = append(cells, layout.Rigid(layout.Spacer{Width: 10}.Layout))
					}
					cells = append(cells, layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
						return t.button(gtx, l.linkButton(it.key), it.label, buttonStyle{kind: btnPlain, wide: true, size: 14})
					}))
				}
				if len(pair) == 1 {
					cells = append(cells, layout.Rigid(layout.Spacer{Width: 10}.Layout), layout.Flexed(1, layout.Spacer{}.Layout))
				}
				return layout.Flex{}.Layout(gtx, cells...)
			})
		}))
	}
	rows = append(rows, layout.Rigid(t.muted("These open in your web browser.").Layout))
	return layout.Flex{Axis: layout.Vertical}.Layout(gtx, rows...)
}

// posts lays news posts out in a scrolling list.
func (l *launcher) posts(gtx layout.Context, list *widget.List, posts []newsPost, s status) layout.Dimensions {
	t := l.th
	if len(posts) == 0 {
		msg := "No news yet."
		if s.live == nil {
			msg = "The news shows once the launcher reaches the server."
		}
		return t.muted(msg).Layout(gtx)
	}
	gtx.Constraints.Min = gtx.Constraints.Max
	return material.List(t.Theme, list).Layout(gtx, len(posts), func(gtx layout.Context, i int) layout.Dimensions {
		p := posts[i]
		return layout.Inset{Right: 12}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
			return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					if i == 0 {
						return layout.Dimensions{}
					}
					return layout.Inset{Top: 14, Bottom: 14}.Layout(gtx, t.rowLine)
				}),
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					if p.Title == "" {
						return layout.Dimensions{}
					}
					return layout.Inset{Bottom: 6}.Layout(gtx, t.text(18, font.ExtraBold, t.p.fg, p.Title).Layout)
				}),
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					lb := t.body(p.Text)
					lb.LineHeightScale = 1.35
					return lb.Layout(gtx)
				}),
			)
		})
	})
}

// --- News tab ---

func (l *launcher) newsTab(gtx layout.Context, s status) layout.Dimensions {
	var posts []newsPost
	if s.live != nil {
		posts = s.live.News
	}
	return l.th.panel(gtx, "News", true, func(gtx layout.Context) layout.Dimensions {
		return l.posts(gtx, &l.lists.news, posts, s)
	})
}

// --- Players tab ---

func (l *launcher) playersTab(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	var online, top []liveCharacter
	count := 0
	note := ""
	switch {
	case s.live == nil:
		note = "Who's online shows once the launcher reaches the server."
	case !s.live.Players:
		note = "The server couldn't read its player list just now."
	default:
		online, top, count = s.live.Online, s.live.Top, s.live.OnlineCount
	}
	onlineTitle := "Who's online"
	if s.live != nil && s.live.Players {
		onlineTitle += " (" + strconv.Itoa(count) + ")"
	}
	return layout.Flex{}.Layout(gtx,
		layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return t.panel(gtx, onlineTitle, true, func(gtx layout.Context) layout.Dimensions {
				empty := note
				if empty == "" {
					empty = "Nobody is playing right now."
				}
				more := ""
				if count > len(online) {
					more = fmt.Sprintf("And %d more.", count-len(online))
				}
				return l.rows(gtx, &l.lists.online, len(online), empty, more, func(gtx layout.Context, i int) layout.Dimensions {
					c := online[i]
					return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
						layout.Flexed(1, t.bold(c.Name).Layout),
						layout.Rigid(t.text(14, font.Normal, t.p.muted, fmt.Sprintf("%s %d/%d", c.Job, c.BaseLevel, c.JobLevel)).Layout),
						layout.Rigid(func(gtx layout.Context) layout.Dimensions {
							if c.Map == "" {
								return layout.Dimensions{}
							}
							return layout.Inset{Left: 12}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
								gtx.Constraints.Min.X = gtx.Dp(90)
								lb := t.text(14, font.Bold, t.p.link, c.Map)
								lb.Alignment = text.End
								return lb.Layout(gtx)
							})
						}),
					)
				})
			})
		}),
		layout.Rigid(layout.Spacer{Width: 16}.Layout),
		layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return t.panel(gtx, "Top adventurers", true, func(gtx layout.Context) layout.Dimensions {
				empty := note
				if empty == "" {
					empty = "No characters yet."
				}
				return l.rows(gtx, &l.lists.top, len(top), empty, "", func(gtx layout.Context, i int) layout.Dimensions {
					c := top[i]
					return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
						layout.Rigid(func(gtx layout.Context) layout.Dimensions {
							gtx.Constraints.Min.X = gtx.Dp(30)
							return t.text(15, font.Bold, t.p.muted, strconv.Itoa(i+1)).Layout(gtx)
						}),
						layout.Flexed(1, t.bold(c.Name).Layout),
						layout.Rigid(t.text(14, font.Normal, t.p.muted, c.Job).Layout),
						layout.Rigid(func(gtx layout.Context) layout.Dimensions {
							return layout.Inset{Left: 12}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
								gtx.Constraints.Min.X = gtx.Dp(28)
								lb := t.bold(strconv.Itoa(c.BaseLevel))
								lb.Alignment = text.End
								return lb.Layout(gtx)
							})
						}),
					)
				})
			})
		}),
	)
}

// rows is a scrolling list with lines between rows, a message when it's
// empty and an optional last line.
func (l *launcher) rows(gtx layout.Context, list *widget.List, n int, empty, last string, row func(gtx layout.Context, i int) layout.Dimensions) layout.Dimensions {
	t := l.th
	if n == 0 {
		return t.muted(empty).Layout(gtx)
	}
	total := n
	if last != "" {
		total++
	}
	gtx.Constraints.Min = gtx.Constraints.Max
	return material.List(t.Theme, list).Layout(gtx, total, func(gtx layout.Context, i int) layout.Dimensions {
		return layout.Inset{Right: 12}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
			if i == n {
				return layout.Inset{Top: 12}.Layout(gtx, t.muted(last).Layout)
			}
			return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					return layout.Inset{Top: 9, Bottom: 9}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
						return row(gtx, i)
					})
				}),
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					if i == n-1 {
						return layout.Dimensions{}
					}
					return t.rowLine(gtx)
				}),
			)
		})
	})
}

// --- Settings tab ---

func (l *launcher) settingsTab(gtx layout.Context, s status) layout.Dimensions {
	t := l.th
	return layout.Flex{Alignment: layout.Start}.Layout(gtx,
		layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return t.panel(gtx, "Game", false, func(gtx layout.Context) layout.Dimensions {
				rows := []layout.FlexChild{
					l.settingRow(&l.setup, "Game settings", false, "Screen size, graphics and sound, in the client's own settings program."),
					l.settingRow(&l.repair, "Check game files", s.busy || l.confirm, "Downloads the server's files again and rebuilds "+defaultGRF+". Use it when items, maps or text look wrong, or after an update was cut off."),
				}
				if l.confirm {
					rows = append(rows, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						return layout.Inset{Bottom: 16}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
							return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
								layout.Flexed(1, t.text(14, font.Bold, t.p.fg, "Download every patch again? It can take a while.").Layout),
								layout.Rigid(func(gtx layout.Context) layout.Dimensions {
									return t.button(gtx, &l.repairOK, "Check files", buttonStyle{kind: btnBlue, size: 14})
								}),
								layout.Rigid(layout.Spacer{Width: 8}.Layout),
								layout.Rigid(func(gtx layout.Context) layout.Dimensions {
									return t.button(gtx, &l.repairNo, "Cancel", buttonStyle{kind: btnPlain, size: 14})
								}),
							)
						})
					}))
				}
				rows = append(rows,
					l.settingRow(&l.folder, "Open game folder", false, l.dir),
					l.settingRow(&l.shots, "Open screenshots", false, "The pictures the game saves when you press Print Screen."),
				)
				return layout.Flex{Axis: layout.Vertical}.Layout(gtx, rows...)
			})
		}),
		layout.Rigid(layout.Spacer{Width: 16}.Layout),
		layout.Flexed(1, func(gtx layout.Context) layout.Dimensions {
			return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					return t.panel(gtx, "Launcher", false, func(gtx layout.Context) layout.Dimensions {
						return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
							layout.Rigid(func(gtx layout.Context) layout.Dimensions {
								cb := material.CheckBox(t.Theme, &l.keepOpen, "Keep the launcher open after Play")
								cb.Color, cb.IconColor = t.p.fg, t.p.link
								return cb.Layout(gtx)
							}),
							layout.Rigid(layout.Spacer{Height: 12}.Layout),
							layout.Rigid(t.text(15, font.ExtraBold, t.p.fg, "Theme").Layout),
							layout.Rigid(func(gtx layout.Context) layout.Dimensions {
								var opts []layout.FlexChild
								for _, o := range []struct{ key, label string }{{themeSystem, "Match Windows"}, {themeLight, "Light"}, {themeDark, "Dark"}} {
									o := o
									opts = append(opts, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
										rb := material.RadioButton(t.Theme, &l.themes, o.key, o.label)
										rb.Color, rb.IconColor = t.p.fg, t.p.link
										return rb.Layout(gtx)
									}), layout.Rigid(layout.Spacer{Width: 12}.Layout))
								}
								return layout.Flex{}.Layout(gtx, opts...)
							}),
						)
					})
				}),
				layout.Rigid(layout.Spacer{Height: 16}.Layout),
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					return t.panel(gtx, "About", false, func(gtx layout.Context) layout.Dimensions {
						return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
							layout.Rigid(t.body("Server: "+l.cfg.Portal).Layout),
							layout.Rigid(layout.Spacer{Height: 6}.Layout),
							layout.Rigid(t.muted("The launcher updates itself and the game from this server. It uses the Nunito font (SIL Open Font License) and Gio; their licences are in the licenses folder of the download.").Layout),
						)
					})
				}),
			)
		}),
	)
}

func (l *launcher) settingRow(c *widget.Clickable, label string, disabled bool, about string) layout.FlexChild {
	t := l.th
	return layout.Rigid(func(gtx layout.Context) layout.Dimensions {
		return layout.Inset{Bottom: 16}.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
			return layout.Flex{Alignment: layout.Middle}.Layout(gtx,
				layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					gtx.Constraints.Min.X = gtx.Dp(170)
					gtx.Constraints.Max.X = gtx.Constraints.Min.X
					return t.button(gtx, c, label, buttonStyle{kind: btnPlain, size: 14, wide: true, disabled: disabled})
				}),
				layout.Rigid(layout.Spacer{Width: 14}.Layout),
				layout.Flexed(1, t.muted(about).Layout),
			)
		})
	})
}

// --- Win32 ---

// shellExecute opens a program, folder or link the way Explorer does, so
// a client that asks for administrator rights gets Windows' prompt.
func shellExecute(file, args, dir string) error {
	verb, _ := windows.UTF16PtrFromString("open")
	f, err := windows.UTF16PtrFromString(file)
	if err != nil {
		return err
	}
	var a, d *uint16
	if args != "" {
		a, _ = windows.UTF16PtrFromString(args)
	}
	if dir != "" {
		d, _ = windows.UTF16PtrFromString(dir)
	}
	r, _, _ := pShellExecute.Call(0, uintptr(unsafe.Pointer(verb)), uintptr(unsafe.Pointer(f)),
		uintptr(unsafe.Pointer(a)), uintptr(unsafe.Pointer(d)), swShow)
	if r <= 32 {
		switch r {
		case 2, 3:
			return errors.New("Windows can't find it")
		case 5:
			return errors.New("Windows refused (access denied); an antivirus may be blocking it")
		case 11:
			return errors.New("it isn't a working Windows program")
		case 31:
			return errors.New("there's no program to open it with")
		}
		return fmt.Errorf("Windows error %d", r)
	}
	return nil
}

const (
	wmClose            = 0x0010
	spiGetWorkArea     = 0x0030
	wsOverlappedWindow = 0x00CF0000
	wsExAppWindow      = 0x00040000
	wsExWindowEdge     = 0x00000100
	monitorPrimary     = 1 // MONITOR_DEFAULTTOPRIMARY
	monitorNearest     = 2 // MONITOR_DEFAULTTONEAREST
	swpNoZOrder        = 0x0004
	swpNoActivate      = 0x0010
	logPixelsX         = 88
)

type dpSize struct{ w, h unit.Dp }

type rect struct{ left, top, right, bottom int32 }

type monitorInfo struct {
	size    uint32
	monitor rect
	work    rect
	flags   uint32
}

// windowSize is the window's size and smallest size in dp: windowW by
// windowH and minW by minH, each shrunk to fit the main screen's work area
// (the screen less the taskbar) with the window's frame around it. Gio
// takes sizes in dp and doesn't fit them to the screen itself, so without
// this 1080p at 150% opens with Play behind the taskbar, and 768p at 125%
// can't fit even the smallest size.
func windowSize() (size, least dpSize) {
	size, least = dpSize{windowW, windowH}, dpSize{minW, minH}
	// Gio does this too, when it opens the window; the sizes here must be
	// in the same pixels.
	pSetProcessDPIAware.Call()
	var work rect
	if r, _, _ := pSystemParametersInfo.Call(spiGetWorkArea, 0, uintptr(unsafe.Pointer(&work)), 0); r == 0 {
		return
	}
	var frame rect
	pAdjustWindowRectEx.Call(uintptr(unsafe.Pointer(&frame)), wsOverlappedWindow, 0, wsExAppWindow|wsExWindowEdge)
	px := float32(systemDPI()) / 96
	fitW := unit.Dp(float32(work.right-work.left-(frame.right-frame.left)) / px)
	fitH := unit.Dp(float32(work.bottom-work.top-(frame.bottom-frame.top)) / px)
	size = dpSize{min(size.w, fitW), min(size.h, fitH)}
	least = dpSize{min(least.w, size.w), min(least.h, size.h)}
	return
}

// systemDPI is the main screen's DPI, worked out as Gio does it.
func systemDPI() int {
	if pGetDpiForMonitor.Find() == nil {
		desktop, _, _ := pGetDesktopWindow.Call()
		mon, _, _ := pMonitorFromWindow.Call(desktop, monitorPrimary)
		var x, y uint32
		if r, _, _ := pGetDpiForMonitor.Call(mon, 0, uintptr(unsafe.Pointer(&x)), uintptr(unsafe.Pointer(&y))); r == 0 && x > 0 {
			return int(x)
		}
	}
	dc, _, _ := pGetDC.Call(0)
	if dc == 0 {
		return 96
	}
	defer pReleaseDC.Call(0, dc)
	if dpi, _, _ := pGetDeviceCaps.Call(dc, logPixelsX); dpi > 0 {
		return int(dpi)
	}
	return 96
}

// fitWindow centres the window in the work area of the screen it opened
// on, shrinking it if that screen is smaller than the main one. Windows
// would place it anywhere, bottom edge under the taskbar included. It runs
// on the window's thread.
func fitWindow(hwnd uintptr) {
	mon, _, _ := pMonitorFromWindow.Call(hwnd, monitorNearest)
	mi := monitorInfo{size: uint32(unsafe.Sizeof(monitorInfo{}))}
	if r, _, _ := pGetMonitorInfoW.Call(mon, uintptr(unsafe.Pointer(&mi))); r == 0 {
		return
	}
	var wr rect
	if r, _, _ := pGetWindowRect.Call(hwnd, uintptr(unsafe.Pointer(&wr))); r == 0 {
		return
	}
	work := mi.work
	w := min(wr.right-wr.left, work.right-work.left)
	h := min(wr.bottom-wr.top, work.bottom-work.top)
	x := work.left + (work.right-work.left-w)/2
	y := work.top + (work.bottom-work.top-h)/2
	pSetWindowPos.Call(hwnd, 0, uintptr(x), uintptr(y), uintptr(w), uintptr(h), swpNoZOrder|swpNoActivate)
}

// messageBox is for the checks before the window opens, and for a window
// that couldn't open.
func messageBox(text string, flags uintptr) int {
	t, _ := windows.UTF16PtrFromString("Launcher")
	m, _ := windows.UTF16PtrFromString(strings.ReplaceAll(text, "\x00", ""))
	r, _, _ := pMessageBoxW.Call(0, uintptr(unsafe.Pointer(m)), uintptr(unsafe.Pointer(t)), flags|mbOK)
	return int(r)
}

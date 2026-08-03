PREFIX        ?= /usr
BIN            = $(DESTDIR)$(PREFIX)/bin
UNITDIR        = $(DESTDIR)$(PREFIX)/lib/systemd/system
DBUSDIR        = $(DESTDIR)$(PREFIX)/share/dbus-1/system.d
POLKIT_ACTIONS = $(DESTDIR)$(PREFIX)/share/polkit-1/actions
APPS           = $(DESTDIR)$(PREFIX)/share/applications
AUTOSTART      = $(DESTDIR)/etc/xdg/autostart
ICONS          = $(DESTDIR)$(PREFIX)/share/icons/hicolor/512x512/apps
TRAY_ICONS     = $(DESTDIR)$(PREFIX)/share/icons/hicolor/64x64/apps

# The tray asks its host for icons by name (legion-toolkit-<profile>), so the
# pre-rendered PNGs must land in the hicolor theme alongside the app icon.
TRAY_ICON_NAMES = legion-toolkit legion-toolkit-quiet legion-toolkit-balanced \
                  legion-toolkit-performance legion-toolkit-extreme \
                  legion-toolkit-custom

.PHONY: build deb install uninstall

build:
	cargo build --release --workspace

deb:
	cargo deb -p legion-daemon

install:
	install -Dm755 target/release/legiond    $(BIN)/legiond
	install -Dm755 target/release/legion-ctl $(BIN)/legion-ctl
	install -Dm755 target/release/legion-gui $(BIN)/legion-gui
	install -Dm644 systemd/legiond.service   $(UNITDIR)/legiond.service
	install -Dm644 dbus/org.legiontoolkit.Daemon.conf $(DBUSDIR)/org.legiontoolkit.Daemon.conf
	install -Dm644 polkit/org.legion-toolkit.policy $(POLKIT_ACTIONS)/org.legiontoolkit.policy
	install -Dm644 desktop/legion-toolkit.desktop $(APPS)/legion-toolkit.desktop
	install -Dm644 desktop/legion-toolkit-tray.desktop $(AUTOSTART)/legion-toolkit-tray.desktop
	install -Dm644 logo.png $(ICONS)/legion-toolkit.png
	for icon in $(TRAY_ICON_NAMES); do \
	    install -Dm644 crates/legion-gui/assets/$$icon.png $(TRAY_ICONS)/$$icon.png; \
	done

uninstall:
	rm -f $(BIN)/legiond $(BIN)/legion-ctl $(BIN)/legion-gui
	rm -f $(UNITDIR)/legiond.service
	rm -f $(DBUSDIR)/org.legiontoolkit.Daemon.conf
	rm -f $(POLKIT_ACTIONS)/org.legiontoolkit.policy
	rm -f $(APPS)/legion-toolkit.desktop
	rm -f $(AUTOSTART)/legion-toolkit-tray.desktop
	rm -f $(ICONS)/legion-toolkit.png
	for icon in $(TRAY_ICON_NAMES); do \
	    rm -f $(TRAY_ICONS)/$$icon.png; \
	done

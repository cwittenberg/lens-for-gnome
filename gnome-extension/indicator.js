import St from 'gi://St';
import GObject from 'gi://GObject';
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

export const GnomeLensIndicator = GObject.registerClass(
    class GnomeLensIndicator extends PanelMenu.Button {
        _init(extension, settings) {
            super._init(0.0, 'Lens for GNOME', false);

            this._extension = extension;
            this._settings = settings;

            // Resolve the path to logo.svg within the extension directory bundle
            let logoFile = this._extension.dir.get_child('logo.svg');
            let icon;

            if (logoFile.query_exists(null)) {
                let fileIcon = new Gio.FileIcon({ file: logoFile });
                icon = new St.Icon({
                    gicon: fileIcon,
                    style_class: 'system-status-icon',
                    icon_size: 16
                });
            } else {
                // Fallback icon in case the SVG asset fails to resolve
                icon = new St.Icon({
                    icon_name: 'system-search-symbolic',
                    style_class: 'system-status-icon'
                });
            }
            
            this.add_child(icon);

            this._buildMenu();

            this._settings.connectObject('changed::search-history', this._buildMenu.bind(this), this);
            this._settings.connectObject('changed::enable-history', this._buildMenu.bind(this), this);

            this.connectObject('captured-event', this._onCapturedEvent.bind(this), this);
            
            this.menu.connectObject('open-state-changed', (menu, isOpen) => {
                if (isOpen) {
                    this._checkServiceStatus();
                }
            }, this);
        }

        _onCapturedEvent(actor, event) {
            let type = event.type();
            if (type !== Clutter.EventType.BUTTON_PRESS && type !== Clutter.EventType.BUTTON_RELEASE) {
                return Clutter.EVENT_PROPAGATE;
            }

            let button = event.get_button();
            if (button === 1 || button === 3) {
                if (type === Clutter.EventType.BUTTON_RELEASE) {
                    if (button === 1) {
                        if (this.menu && this.menu.isOpen) {
                            this.menu.close();
                        }
                        this._extension.toggleLens();
                    } else if (button === 3) {
                        this._buildMenu();
                        if (this.menu) {
                            this.menu.toggle();
                        }
                    }
                }
                
                return Clutter.EVENT_STOP;
            }

            return Clutter.EVENT_PROPAGATE;
        }

        _checkServiceStatus() {
            let socketClient = new Gio.SocketClient();
            let socketPath = GLib.get_home_dir() + '/.local/state/lens-for-gnome/lens_for_gnome.sock';
            let address = Gio.UnixSocketAddress.new(socketPath);

            socketClient.connect_async(address, null, (client, res) => {
                try {
                    let conn = client.connect_finish(res);
                    conn.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                    this._statusItem.label.set_text('🟢 Service: Online');
                } catch (e) {
                    this._statusItem.label.set_text('🔴 Service: Offline');
                }
            });
        }

        _buildMenu() {
            this.menu.removeAll();

            this._statusItem = new PopupMenu.PopupMenuItem('🟡 Service: Checking...', { reactive: false });
            this.menu.addMenuItem(this._statusItem);

            this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

            if (this._settings.get_boolean('enable-history')) {
                let history = this._settings.get_strv('search-history');
                let historySection = new PopupMenu.PopupMenuSection();

                if (history.length === 0) {
                    let emptyItem = new PopupMenu.PopupMenuItem('No recent searches');
                    emptyItem.setSensitive(false);
                    historySection.addMenuItem(emptyItem);
                } else {
                    for (let query of history) {
                        let item = new PopupMenu.PopupMenuItem(query);
                        item.connectObject('activate', () => {
                            this._extension.openLensWithQuery(query);
                        }, this);
                        historySection.addMenuItem(item);
                    }
                }

                this.menu.addMenuItem(historySection);
                this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
            }

            let prefsItem = new PopupMenu.PopupImageMenuItem('Preferences', 'preferences-system-symbolic');
            prefsItem.connectObject('activate', () => {
                this._extension.openPreferences();
            }, this);
            this.menu.addMenuItem(prefsItem);
        }

        destroy() {
            this._settings.disconnectObject(this);
            this.disconnectObject(this);
            super.destroy();
        }
    }
);
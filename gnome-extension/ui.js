// gnome-extension/ui.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import ServiceClient from './service.js';
import { GnomeLensSearchBar } from './ui_search.js';
import { GnomeLensResultsList } from './ui_results.js';
import { GnomeLensSynthesis, GnomeLensStatus } from './ui_status.js';

export const GnomeLensUI = GObject.registerClass({
    GTypeName: 'GnomeLensUI',
}, class GnomeLensUI extends St.Widget {

    _init(settings, extension) {
        super._init({
            name: 'GnomeLensBackdrop',
            style_class: 'lens-backdrop',
            reactive: true,
            can_focus: true,
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });

        this._settings = settings;
        this._extension = extension;
        this._service = new ServiceClient();
        this._historyIndex = -1;
        this._modalGrab = null;
        this._modalPushed = false;
        this._stageCaptureConnected = false;
        
        this.isOpen = false;
        this.isClosing = false;

        this._buildUI();

        this.connectObject('button-press-event', () => {
            this.close();
            return Clutter.EVENT_STOP;
        }, this);

        Main.layoutManager.connectObject('monitors-changed', this._onMonitorsChanged.bind(this), this);

        // React to style changes in real-time
        this._settings.connectObject(
            'changed::ui-color', this._applyStyles.bind(this),
            'changed::ui-transparency', this._applyStyles.bind(this),
            'changed::ui-shadow', this._applyStyles.bind(this),
            'changed::show-document-text', () => {
                if (this._resultsList && this._resultsList.hasResults()) {
                    this._resultsList.renderResults([...this._resultsList.getResults()]);
                }
            },
            this
        );
        this._applyStyles();
    }

    _applyStyles() {
        let color = this._settings.get_string('ui-color');
        let opacity = this._settings.get_int('ui-transparency') / 100.0;
        let shadow = this._settings.get_boolean('ui-shadow');
        
        let r = 30, g = 30, b = 30;
        if (/^#[0-9A-Fa-f]{6}$/.test(color)) {
            r = parseInt(color.slice(1, 3), 16);
            g = parseInt(color.slice(3, 5), 16);
            b = parseInt(color.slice(5, 7), 16);
        }

        let shadowCss = shadow ? 'box-shadow: 0px 15px 50px rgba(0, 0, 0, 0.5);' : 'box-shadow: none;';
        let bgCss = `background-color: rgba(${r}, ${g}, ${b}, ${opacity});`;
        
        this._dialog.set_style(`${bgCss} ${shadowCss}`);
    }

    _getAnimationParams(baseDuration, isClose = false) {
        if (!this._settings.get_boolean('ui-animation')) {
            return { duration: 0, mode: Clutter.AnimationMode.EASE_OUT_QUAD };
        }
        let type = this._settings.get_string('ui-animation-type');
        let mode = isClose ? Clutter.AnimationMode.EASE_IN_QUAD : Clutter.AnimationMode.EASE_OUT_QUAD;
        
        if (type === 'bounce') {
            mode = isClose ? Clutter.AnimationMode.EASE_IN_BOUNCE : Clutter.AnimationMode.EASE_OUT_BOUNCE;
        } else if (type === 'elastic') {
            mode = isClose ? Clutter.AnimationMode.EASE_IN_ELASTIC : Clutter.AnimationMode.EASE_OUT_ELASTIC;
        }
        
        return { duration: baseDuration, mode: mode };
    }

    _buildUI() {
        this._dialog = new St.BoxLayout({
            vertical: true,
            style_class: 'lens-dialog',
            reactive: true,
        });
        
        this._dialog.set_pivot_point(0.5, 0.5);
        this._dialog.set_scale(0.9, 0.9);
        this._dialog.set_opacity(0);

        this._dialog.connectObject('button-press-event', () => {
            return Clutter.EVENT_STOP;
        }, this);

        this._searchBar = new GnomeLensSearchBar(this._settings, {
            onClose: () => this.close(true),
            onClear: () => {
                this._service.cancel();
                this._resultsList.clear();
                this._synthesis.setSynthesis(null);
                this._status.stopAnimation();
                this._status.setStatus('');
                this._searchBar.stopPulse();
                this._searchBar.setCount(0);
                this._updatePosition(false, true);
            },
            onSearch: (text) => {
                this._triggerBackendSearch(text);
            },
            onNavigateUp: () => {
                if (this._resultsList.getSelectedIndex() > 0) {
                    this._resultsList.selectPrev();
                } else if (this._resultsList.getSelectedIndex() === 0) {
                    this._resultsList.selectPrev();
                } else if (this._resultsList.getSelectedIndex() === -1) {
                    let history = this._settings.get_strv('search-history') || [];
                    if (this._historyIndex < history.length - 1) {
                        this._historyIndex++;
                        this._loadHistoryAt(this._historyIndex);
                    }
                }
            },
            onNavigateDown: () => {
                if (this._resultsList.hasResults() && this._resultsList.getSelectedIndex() < this._resultsList.getCount() - 1) {
                    this._resultsList.selectNext();
                } else if (this._resultsList.getSelectedIndex() === -1) {
                    if (this._historyIndex > 0) {
                        this._historyIndex--;
                        this._loadHistoryAt(this._historyIndex);
                    } else if (this._historyIndex === 0) {
                        this._historyIndex = -1;
                        this._searchBar.setQuery('', false);
                    }
                }
            },
            onNavigateEnter: (query) => {
                if (this._resultsList.getSelectedIndex() !== -1) {
                    this._resultsList.launchSelected();
                } else if (query.length > 0) {
                    this._extension.saveHistory(query);
                }
            }
        });
        this._dialog.add_child(this._searchBar);

        this._resultsList = new GnomeLensResultsList(this._settings, {
            onLaunch: (result, action) => this._launchResult(result, action)
        });
        this._dialog.add_child(this._resultsList);

        this._synthesis = new GnomeLensSynthesis();
        this._resultsList.addSynthesisWidget(this._synthesis);

        this._status = new GnomeLensStatus(this._settings);
        this._dialog.add_child(this._status);

        this.add_child(this._dialog);
        this._updatePosition(false, false);
    }

    _getActiveMonitor() {
        let [x, y] = global.get_pointer();
        
        let monitors = Main.layoutManager.monitors;
        let activeMonitorIndex = monitors.findIndex(m => 
            x >= m.x && x < m.x + m.width &&
            y >= m.y && y < m.y + m.height
        );

        if (activeMonitorIndex >= 0) {
            return monitors[activeMonitorIndex];
        }

        return Main.layoutManager.primaryMonitor;
    }

    _updatePosition(hasResults = false, animate = true) {
        let monitor = this._getActiveMonitor();
        
        this.set_position(monitor.x, monitor.y);
        this.set_size(monitor.width, monitor.height);

        let dialogWidth = Math.min(1560, Math.floor(monitor.width * 0.85));
        this._dialog.set_width(dialogWidth);

        let maxScrollHeight = Math.min(700, Math.floor(monitor.height * 0.75));
        this._resultsList.style = `max-height: ${maxScrollHeight}px;`;

        let targetX = Math.floor((monitor.width - dialogWidth) / 2);
        let targetY = hasResults
            ? Math.floor(monitor.height * 0.20)
            : Math.floor(monitor.height * 0.40);

        this._dialog.remove_transition('x');
        this._dialog.remove_transition('y');

        let anim = this._getAnimationParams(250, false);

        if (animate && anim.duration > 0) {
            this._dialog.ease({
                x: targetX,
                y: targetY,
                duration: anim.duration,
                mode: anim.mode,
            });
        } else {
            this._dialog.set_position(targetX, targetY);
        }
    }

    _onMonitorsChanged() {
        this._updatePosition(this._resultsList.hasResults(), false);
    }

    _connectStageCapture() {
        if (this._stageCaptureConnected) return;
        global.stage.connectObject('captured-event', this._onCapturedEvent.bind(this), this);
        this._stageCaptureConnected = true;
    }

    _disconnectStageCapture() {
        if (!this._stageCaptureConnected) return;
        global.stage.disconnectObject(this);
        this._stageCaptureConnected = false;
    }

    _onCapturedEvent(actor, event) {
        if (!this.isOpen || this.isClosing) {
            return Clutter.EVENT_PROPAGATE;
        }

        if (event.type() === Clutter.EventType.KEY_PRESS) {
            let symbol = event.get_key_symbol();
            if (symbol === Clutter.KEY_Escape) {
                this.close(true);
                return Clutter.EVENT_STOP;
            }
        }

        return Clutter.EVENT_PROPAGATE;
    }

    _pushModal() {
        let grab = Main.pushModal(this);
        this._modalPushed = !!grab;
        this._modalGrab = grab && grab !== true ? grab : null;
    }

    _popModal() {
        if (!this._modalPushed && !this._modalGrab) return;
        
        let grab = this._modalGrab;
        this._modalGrab = null;
        this._modalPushed = false;

        if (grab) {
            Main.popModal(grab);
        } else {
            Main.popModal(this);
        }
    }

    open() {
        if (this.isOpen || this.isClosing) return;

        this.isOpen = true;
        this.isClosing = false;
        
        this.show();
        this.reactive = true;
        this._dialog.reactive = true;

        if (!this.get_parent()) {
            Main.layoutManager.uiGroup.add_child(this);
        }

        this._pushModal();
        this._connectStageCapture();
        
        this._historyIndex = -1;
        this._updatePosition(this._resultsList.hasResults(), false);
        
        this._dialog.remove_all_transitions();
        
        let anim = this._getAnimationParams(150, false);
        if (anim.duration > 0) {
            this._dialog.set_scale(0.9, 0.9);
            this._dialog.set_opacity(0);
            this._dialog.ease({
                scale_x: 1.0,
                scale_y: 1.0,
                opacity: 255,
                duration: anim.duration,
                mode: anim.mode,
            });
        } else {
            this._dialog.set_scale(1.0, 1.0);
            this._dialog.set_opacity(255);
        }

        this.grab_key_focus();
        this._searchBar.grabFocus();
    }

    close(instant = false) {
        if (this.isClosing || !this.isOpen) return;

        this.isClosing = true;
        this.reactive = false;
        this._dialog.reactive = false;
        
        this._service.cancel();
        this._status.stopAnimation();
        this._searchBar.stopPulse();

        this._disconnectStageCapture();
        global.stage.set_key_focus(null);
        this._popModal();

        this.isOpen = false;

        if (instant) {
            this._finishClose();
            return;
        }

        let anim = this._getAnimationParams(100, true);
        if (anim.duration > 0) {
            this._dialog.remove_all_transitions();
            this._dialog.ease({
                scale_x: 0.9,
                scale_y: 0.9,
                opacity: 0,
                duration: anim.duration,
                mode: anim.mode,
                onComplete: () => {
                    this._finishClose();
                },
            });
        } else {
            this._finishClose();
        }
    }

    _finishClose() {
        this.hide();
        this._dialog.remove_all_transitions();
        this._dialog.set_scale(0.9, 0.9);
        this._dialog.set_opacity(0);

        if (this.get_parent()) {
            Main.layoutManager.uiGroup.remove_child(this);
        }
        this.isClosing = false;
    }

    setQuery(text) {
        this._searchBar.setQuery(text);
    }

    vfunc_key_press_event(keyEvent) {
        if (keyEvent.get_key_symbol() === Clutter.KEY_Escape) {
            this.close(true);
            return Clutter.EVENT_STOP;
        }
        return super.vfunc_key_press_event(keyEvent);
    }

    _loadHistoryAt(index) {
        let history = this._settings.get_strv('search-history') || [];
        if (index >= 0 && index < history.length) {
            this._searchBar.setQuery(history[index], false);
        }
    }

    _launchResult(result, action = 'open') {
        this._extension.saveHistory(this._searchBar.getQuery());
        this.close(true);

        // Chain of Responsibility: Isolated listener for delegation. 
        // This persists even after the main UI dialog is destroyed.
        let delegationCallback = {
            onMessage: (data) => {
                if (data.status === 'delegate') {
                    console.log(`[Gnome Lens] [IPC Chain] Stage 3: Received delegation request from backend for ${data.action} -> ${data.path}`);
                    try {
                        let targetPath = data.path;
                        if (data.action === 'open_folder') {
                            let lastSlash = targetPath.lastIndexOf('/');
                            targetPath = lastSlash > 0 ? targetPath.substring(0, lastSlash) : '/';
                        }
                        
                        // Safety Trap: Prevent GNOME from throwing a hard Exception if the file was deleted post-index
                        if (!GLib.file_test(targetPath, GLib.FileTest.EXISTS)) {
                            console.warn(`[Gnome Lens] [IPC Chain] Abort: The file or directory no longer exists on disk: ${targetPath}`);
                            return;
                        }

                        let file = Gio.File.new_for_path(targetPath);
                        let uri = file.get_uri();
                        
                        console.log(`[Gnome Lens] [IPC Chain] Stage 4: Executing ultimate Wayland fallback via Gio.AppInfo for: ${uri}`);
                        
                        // EGO-Compliant Asynchronous Ultimate Fallback
                        // Completely non-blocking to protect the Wayland compositor loop
                        Gio.AppInfo.launch_default_for_uri_async(
                            uri, 
                            null, 
                            null, 
                            (appInfo, res) => {
                                try {
                                    Gio.AppInfo.launch_default_for_uri_finish(res);
                                    console.log(`[Gnome Lens] [IPC Chain] Stage 5: GNOME fallback launch successful.`);
                                } catch (e) {
                                    console.warn(`[Gnome Lens] [IPC Chain] Error: Native async launch failed: ${e}`);
                                }
                            }
                        );
                    } catch (e) {
                        console.warn(`[Gnome Lens] [IPC Chain] Error: Delegation to GNOME Shell failed: ${e}`);
                    }
                }
            },
            onError: (e) => console.warn(`[Gnome Lens] [IPC Chain] Launch IPC error: ${e}`),
            onOffline: () => console.warn(`[Gnome Lens] [IPC Chain] Daemon offline during launch.`)
        };

        console.log(`[Gnome Lens] [IPC Chain] Stage 0: Dispatching launch request to Rust backend...`);

        if (result.plugin_id === 'plugin:app_launcher' && result.metadata && result.metadata.exec) {
            this._service.sendPayload({ action: 'launch_app', exec: result.metadata.exec, filepath: result.filepath || '' }, delegationCallback);
            return;
        }

        if (result.filepath) {
            if (action === 'folder') {
                this._service.sendPayload({ action: 'open_folder', path: result.filepath }, delegationCallback);
            } else {
                this._service.sendPayload({ action: 'open_file', path: result.filepath }, delegationCallback);
            }
        }
    }

    _triggerBackendSearch(query) {
        this._service.cancel();
        this._searchBar.startPulse();

        let filterStrategy = this._settings.get_string('ai-filter-strategy');
        let prioritizeFolders = this._settings.get_boolean('prioritize-folders');

        this._service.search(query, filterStrategy, prioritizeFolders, {
            onMessage: (data) => {
                if (data.status === 'error') {
                    this._status.setStatus(data.message);
                    this._status.stopAnimation();
                    this._searchBar.stopPulse();
                } else if (data.status === 'filtering' || data.status === 'synthesizing' || data.status === 'processing') {
                    this._status.startAnimation(data.message);
                } else if (data.status === 'done' || data.status === 'final') {
                    this._status.stopAnimation();
                    this._searchBar.stopPulse();
                }

                if (data.results && Array.isArray(data.results)) {
                    this._resultsList.renderResults(data.results);
                    this._searchBar.setCount(data.results.length);
                    
                    if (data.results.length > 0) {
                        this._updatePosition(true, true);
                    }

                    if (data.mode === 'rag_synthesis' && data.synthesis_result) {
                        this._synthesis.setSynthesis(data.synthesis_result);
                    }
                }
            },
            onOffline: () => {
                this._status.setStatus('Service offline or unreachable.');
                this._searchBar.stopPulse();
            },
            onError: () => {
                this._searchBar.stopPulse();
            }
        });
    }

    destroy() {
        this._disconnectStageCapture();
        this._popModal();

        if (this.isOpen || this.isClosing) {
            this.isOpen = false;
            this.isClosing = false;
            global.stage.set_key_focus(null);
            if (this.get_parent()) {
                Main.layoutManager.uiGroup.remove_child(this);
            }
        }

        this._service.cancel();
        this._settings.disconnectObject(this);
        this.disconnectObject(this);
        Main.layoutManager.disconnectObject(this);
        super.destroy();
    }
});
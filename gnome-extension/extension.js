import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

const GnomeLensUI = GObject.registerClass({
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
            width: Main.layoutManager.primaryMonitor.width,
            height: Main.layoutManager.primaryMonitor.height,
        });

        this._settings = settings;
        this._extension = extension;
        this._cancellable = null;
        this._socketClient = null;
        this._inputStream = null;
        this._outputStream = null;

        this._results = [];
        this._resultWidgets = [];
        this._selectedIndex = -1;
        this._hasResults = false;
        this._historyIndex = -1;

        this._llmTimerId = 0;
        this._llmDotCount = 0;
        this._activeStatusText = '';
        this._debounceId = 0;

        this._modalGrab = null;
        this._modalPushed = false;
        this._stageCaptureConnected = false;

        this.isOpen = false;
        this.isClosing = false;

        this._thumbnailsCacheDir = GLib.build_filenamev([GLib.get_user_cache_dir(), 'gnome-lens', 'thumbnails']);
        let dirFile = Gio.File.new_for_path(this._thumbnailsCacheDir);
        if (!dirFile.query_exists(null)) {
            try { dirFile.make_directory_with_parents(null); } catch(e) {}
        }

        this._buildUI();

        this.connectObject('button-press-event', () => {
            this.close();
            return Clutter.EVENT_STOP;
        }, this);

        Main.layoutManager.connectObject('monitors-changed', this._onMonitorsChanged.bind(this), this);
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

        let entryContainer = new St.BoxLayout({
            style_class: 'lens-entry-container',
            vertical: false,
        });

        this._entry = new St.Entry({
            style_class: 'lens-entry',
            hint_text: 'Search files, ask the AI...',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            can_focus: true,
        });

        this._entry.clutter_text.connectObject('text-changed', this._onSearchTextChanged.bind(this), this);
        this._entry.clutter_text.connectObject('key-press-event', this._onKeyPress.bind(this), this);
        entryContainer.add_child(this._entry);

        this._closeButton = new St.Button({
            style_class: 'lens-close-button',
            child: new St.Icon({ icon_name: 'window-close-symbolic', icon_size: 24 }),
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            can_focus: true,
        });

        this._closeButton.connectObject('button-press-event', () => {
            this.close(true);
            return Clutter.EVENT_STOP;
        }, this);

        this._closeButton.connectObject('clicked', () => {
            this.close(true);
        }, this);

        entryContainer.add_child(this._closeButton);

        this._dialog.add_child(entryContainer);

        this._scrollView = new St.ScrollView({
            style_class: 'lens-results-scroll',
            x_expand: true,
            y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
        });

        this._resultsBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
        });

        this._scrollView.add_child(this._resultsBox);
        this._dialog.add_child(this._scrollView);

        this._synthesisBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            visible: false,
        });
        this._synthesisLabel = new St.Label({
            style_class: 'lens-synthesis-text',
            x_expand: true,
        });
        this._synthesisLabel.clutter_text.line_wrap = true;
        this._synthesisBox.add_child(this._synthesisLabel);
        this._resultsBox.add_child(this._synthesisBox);

        this._statusContainer = new St.BoxLayout({ style_class: 'lens-status-container', visible: false });
        this._statusLabel = new St.Label({ style_class: 'lens-status-label', text: '' });
        this._statusContainer.add_child(this._statusLabel);
        this._dialog.add_child(this._statusContainer);

        this.add_child(this._dialog);
        this._updatePosition(false, false);
    }

    _updatePosition(hasResults = false, animate = true) {
        this._hasResults = hasResults;
        let monitor = Main.layoutManager.primaryMonitor;
        
        this.set_position(monitor.x, monitor.y);
        this.set_size(monitor.width, monitor.height);

        let dialogWidth = 850;
        this._dialog.set_width(dialogWidth);

        let targetX = Math.floor((monitor.width - dialogWidth) / 2);

        let targetY = hasResults
            ? Math.floor(monitor.height * 0.20)
            : Math.floor(monitor.height * 0.40);

        this._dialog.remove_transition('x');
        this._dialog.remove_transition('y');

        if (animate) {
            this._dialog.ease({
                x: targetX,
                y: targetY,
                duration: 250,
                mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            });
        } else {
            this._dialog.set_position(targetX, targetY);
        }
    }

    _onMonitorsChanged() {
        this._updatePosition(this._hasResults, false);
    }

    _connectStageCapture() {
        if (this._stageCaptureConnected) {
            return;
        }

        global.stage.connectObject('captured-event', this._onCapturedEvent.bind(this), this);
        this._stageCaptureConnected = true;
    }

    _disconnectStageCapture() {
        if (!this._stageCaptureConnected) {
            return;
        }

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

        if (!this._modalPushed) {
            console.warn('[Gnome Lens] Main.pushModal() failed. Input may not be grabbed.');
        }
    }

    _popModal() {
        if (!this._modalPushed && !this._modalGrab) {
            return;
        }

        let grab = this._modalGrab;
        this._modalGrab = null;
        this._modalPushed = false;

        try {
            if (grab) {
                Main.popModal(grab);
            } else {
                Main.popModal(this);
            }
        } catch (error) {
            console.warn(`[Gnome Lens] Failed to pop modal grab cleanly: ${error}`);
        }
    }

    open() {
        if (this.isOpen || this.isClosing) {
            return;
        }

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
        this._updatePosition(this._hasResults, false);

        this._dialog.remove_all_transitions();
        this._dialog.set_scale(0.9, 0.9);
        this._dialog.set_opacity(0);
        this._dialog.ease({
            scale_x: 1.0,
            scale_y: 1.0,
            opacity: 255,
            duration: 150,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });

        if (this._entry.get_text().length > 0) {
            this._entry.clutter_text.set_selection(0, -1);
        }

        this.grab_key_focus();
        this._entry.grab_key_focus();
        this._entry.clutter_text.grab_key_focus();
    }

    close(instant = false) {
        if (this.isClosing || !this.isOpen) {
            return;
        }

        this.isClosing = true;

        this.reactive = false;
        this._dialog.reactive = false;

        this._cancelBackendRequest();
        this._stopAnimation();

        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }

        this._disconnectStageCapture();
        global.stage.set_key_focus(null);
        this._popModal();

        this.isOpen = false;

        if (instant) {
            this._finishClose();
            return;
        }

        this._dialog.remove_all_transitions();
        this._dialog.ease({
            scale_x: 0.9,
            scale_y: 0.9,
            opacity: 0,
            duration: 100,
            mode: Clutter.AnimationMode.EASE_IN_QUAD,
            onComplete: () => {
                this._finishClose();
            },
        });
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
        this._entry.set_text(text);
    }

    vfunc_key_press_event(keyEvent) {
        if (keyEvent.get_key_symbol() === Clutter.KEY_Escape) {
            this.close(true);
            return Clutter.EVENT_STOP;
        }

        return super.vfunc_key_press_event(keyEvent);
    }

    _onKeyPress(actor, event) {
        let symbol = event.get_key_symbol();

        if (symbol === Clutter.KEY_Escape) {
            this.close(true);
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Down) {
            if (this._results.length > 0 && this._selectedIndex < this._results.length - 1) {
                this._setSelectedIndex(this._selectedIndex + 1);
            } else if (this._selectedIndex === -1) {
                if (this._historyIndex > 0) {
                    this._historyIndex--;
                    this._loadHistoryAt(this._historyIndex);
                } else if (this._historyIndex === 0) {
                    this._historyIndex = -1;
                    this._entry.set_text('');
                }
            }
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Up) {
            if (this._selectedIndex > 0) {
                this._setSelectedIndex(this._selectedIndex - 1);
            } else if (this._selectedIndex === 0) {
                this._setSelectedIndex(-1);
            } else if (this._selectedIndex === -1) {
                let history = this._settings.get_strv('search-history') || [];
                if (this._historyIndex < history.length - 1) {
                    this._historyIndex++;
                    this._loadHistoryAt(this._historyIndex);
                }
            }
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
            if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
                this._launchResult(this._results[this._selectedIndex]);
            } else {
                this._extension.saveHistory(this._entry.get_text());
            }
            return Clutter.EVENT_STOP;
        }

        return Clutter.EVENT_PROPAGATE;
    }

    _loadHistoryAt(index) {
        let history = this._settings.get_strv('search-history') || [];
        if (index >= 0 && index < history.length) {
            let query = history[index];
            this._entry.set_text(query);
            GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                this._entry.clutter_text.set_selection(-1, -1);
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    _setSelectedIndex(index) {
        if (this._selectedIndex >= 0 && this._selectedIndex < this._resultWidgets.length) {
            this._resultWidgets[this._selectedIndex].remove_style_class_name('selected');
        }

        this._selectedIndex = index;

        if (this._selectedIndex >= 0 && this._selectedIndex < this._resultWidgets.length) {
            let widget = this._resultWidgets[this._selectedIndex];
            widget.add_style_class_name('selected');

            let adjustment = this._scrollView.vscroll.adjustment;
            let [val, lower, upper, step, page, size] = adjustment.get_values();
            let y = widget.allocation.y1;
            let height = widget.allocation.y2 - widget.allocation.y1;

            if (y < val) {
                adjustment.set_value(y);
            } else if (y + height > val + page) {
                adjustment.set_value(y + height - page);
            }
        }
    }

    _launchResult(result) {
        this._extension.saveHistory(this._entry.get_text());

        let uri = null;

        if (result.filepath) {
            let file = Gio.File.new_for_path(result.filepath);
            uri = file.get_uri();
        }

        this.close(true);

        if (!uri) {
            return;
        }

        GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            let launchContext = global.create_app_launch_context(0, -1);

            Gio.AppInfo.launch_default_for_uri_async(uri, launchContext, null, (_source, res) => {
                try {
                    Gio.AppInfo.launch_default_for_uri_finish(res);
                } catch (error) {
                    console.warn(`[Gnome Lens] Failed to launch result ${uri}: ${error}`);
                }
            });

            return GLib.SOURCE_REMOVE;
        });
    }

    _onSearchTextChanged() {
        let text = this._entry.get_text().trim();

        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }

        if (text.length < 3) {
            this._cancelBackendRequest();
            this._clearResults();
            this._stopAnimation();
            return;
        }

        this._debounceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            this._debounceId = 0;
            this._triggerBackendSearch(text);
            return GLib.SOURCE_REMOVE;
        });
    }

    _clearResults() {
        this._results = [];
        this._selectedIndex = -1;
        this._synthesisBox.hide();
        this._synthesisLabel.set_text('');

        for (let widget of this._resultWidgets) {
            widget.reactive = false;
            widget.remove_all_transitions();
            widget.destroy();
        }
        this._resultWidgets = [];

        this._updatePosition(false, true);
    }

    _cancelBackendRequest() {
        if (this._cancellable) {
            this._cancellable.cancel();
            this._cancellable = null;
        }

        if (this._inputStream) {
            this._inputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
            this._inputStream = null;
        }

        if (this._outputStream) {
            this._outputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
            this._outputStream = null;
        }

        if (this._socketClient) {
            this._socketClient = null;
        }
    }

    _triggerBackendSearch(query) {
        this._cancelBackendRequest();
        this._cancellable = new Gio.Cancellable();
        this._socketClient = new Gio.SocketClient();

        let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
        let address = Gio.UnixSocketAddress.new(socketPath);

        this._socketClient.connect_async(address, this._cancellable, (client, res) => {
            let connection;
            try {
                connection = client.connect_finish(res);
            } catch (error) {
                this._setStatus('Daemon offline or unreachable.');
                return;
            }

            this._outputStream = connection.get_output_stream();
            this._inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });

            let payload = JSON.stringify({ query: query }) + '\n';

            this._outputStream.write_all_async(payload, GLib.PRIORITY_DEFAULT, this._cancellable, (stream, writeRes) => {
                try {
                    stream.write_all_finish(writeRes);
                } catch (error) {
                    return;
                }
                this._readStream();
            });
        });
    }

    _readStream() {
        if (!this._inputStream || !this._cancellable || this._cancellable.is_cancelled()) {
            return;
        }

        this._inputStream.read_line_async(GLib.PRIORITY_DEFAULT, this._cancellable, (stream, res) => {
            let lineData;
            try {
                lineData = stream.read_line_finish_utf8(res);
            } catch (error) {
                return;
            }

            if (!lineData) {
                return;
            }

            let [line] = lineData;
            if (line) {
                try {
                    let parsed = JSON.parse(line);
                    this._handleBackendMessage(parsed);
                } catch (error) {
                    console.warn(`[Gnome Lens] Ignoring invalid daemon JSON: ${error}`);
                }
            }

            this._readStream();
        });
    }

    _handleBackendMessage(data) {
        if (data.status === 'error') {
            this._setStatus(data.message);
            this._stopAnimation();
            return;
        }

        if (data.status === 'filtering' || data.status === 'synthesizing' || data.status === 'processing') {
            this._startAnimation(data.message);
            return;
        }

        if (data.status === 'done') {
            this._stopAnimation();
            return;
        }

        if (data.results && Array.isArray(data.results)) {
            this._renderResults(data.results);

            if (data.status === 'final') {
                this._stopAnimation();
            }

            if (data.mode === 'rag_synthesis' && data.synthesis_text) {
                this._synthesisLabel.set_text(data.synthesis_text);
                this._synthesisBox.show();
            }
        }
    }

    _fetchVideoThumbnail(filepath, iconActor) {
        let hash = GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, filepath, -1);
        let thumbPath = GLib.build_filenamev([this._thumbnailsCacheDir, hash + '.png']);
        let thumbFile = Gio.File.new_for_path(thumbPath);

        thumbFile.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (file, res) => {
            try {
                file.query_info_finish(res);
                iconActor.set_gicon(new Gio.FileIcon({ file: file }));
                iconActor.add_style_class_name('lens-result-preview');
                iconActor.remove_style_class_name('lens-result-icon');
            } catch (e) {
                this._generateVideoThumbnail(filepath, thumbPath, iconActor, thumbFile);
            }
        });
    }

    _generateVideoThumbnail(filepath, thumbPath, iconActor, thumbFile) {
        let argv = ['ffmpegthumbnailer', '-i', filepath, '-o', thumbPath, '-t', '10', '-s', '256'];
        
        try {
            let proc = Gio.Subprocess.new(argv, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            proc.wait_check_async(null, (obj, res) => {
                try {
                    if (obj.wait_check_finish(res)) {
                        iconActor.set_gicon(new Gio.FileIcon({ file: thumbFile }));
                        iconActor.add_style_class_name('lens-result-preview');
                        iconActor.remove_style_class_name('lens-result-icon');
                    }
                } catch (err) {
                    this._generateVideoThumbnailFallback(filepath, thumbPath, iconActor, thumbFile);
                }
            });
        } catch (spawnErr) {
            this._generateVideoThumbnailFallback(filepath, thumbPath, iconActor, thumbFile);
        }
    }

    _generateVideoThumbnailFallback(filepath, thumbPath, iconActor, thumbFile) {
        let argv = ['totem-video-thumbnailer', '-s', '256', filepath, thumbPath];
        try {
            let proc = Gio.Subprocess.new(argv, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            proc.wait_check_async(null, (obj, res) => {
                try {
                    if (obj.wait_check_finish(res)) {
                        iconActor.set_gicon(new Gio.FileIcon({ file: thumbFile }));
                        iconActor.add_style_class_name('lens-result-preview');
                        iconActor.remove_style_class_name('lens-result-icon');
                    }
                } catch (err) {
                }
            });
        } catch (spawnErr) {
        }
    }

    _renderResults(resultsArray) {
        this._clearResults();
        this._results = resultsArray;

        if (resultsArray.length > 0) {
            this._updatePosition(true, true);
        }

        for (let i = 0; i < resultsArray.length; i++) {
            let res = resultsArray[i];

            let itemBox = new St.BoxLayout({
                style_class: 'lens-result-item',
                vertical: false,
                reactive: true,
            });

            itemBox.connectObject('button-press-event', () => {
                this._launchResult(res);
                return Clutter.EVENT_STOP;
            }, this);

            itemBox.connectObject('enter-event', () => {
                this._setSelectedIndex(i);
            }, this);

            let isImagePreview = false;
            let isVideoPreview = false;
            let iconActor;

            let iconName = 'text-x-generic-symbolic';
            
            if (res.metadata && res.metadata.filetype && res.filepath) {
                let ext = res.metadata.filetype.toLowerCase();
                if (['png', 'jpg', 'jpeg', 'bmp', 'webp', 'svg'].includes(ext)) {
                    isImagePreview = true;
                } else if (['mp4', 'mkv', 'webm', 'avi'].includes(ext)) {
                    isVideoPreview = true;
                    iconName = 'video-x-generic-symbolic';
                } else if (['pdf'].includes(ext)) {
                    iconName = 'x-office-document-symbolic';
                } else if (['xlsx', 'csv'].includes(ext)) {
                    iconName = 'x-office-spreadsheet-symbolic';
                }
            }

            if (res.plugin_id === 'plugin:email') iconName = 'mail-unread-symbolic';
            if (res.plugin_id === 'plugin:math') iconName = 'accessories-calculator-symbolic';

            if (isImagePreview && res.filepath) {
                let file = Gio.File.new_for_path(res.filepath);
                iconActor = new St.Icon({
                    gicon: new Gio.FileIcon({ file: file }),
                    style_class: 'lens-result-preview',
                });
            } else {
                iconActor = new St.Icon({
                    icon_name: iconName,
                    style_class: 'lens-result-icon',
                });
                
                if (isVideoPreview && res.filepath) {
                    this._fetchVideoThumbnail(res.filepath, iconActor);
                }
            }

            itemBox.add_child(iconActor);

            let textBox = new St.BoxLayout({
                vertical: true,
                style_class: 'lens-result-text-box',
                y_align: Clutter.ActorAlign.CENTER,
            });

            let title = new St.Label({
                text: res.title || 'Unknown Document',
                style_class: 'lens-result-title',
            });
            textBox.add_child(title);

            if (res.snippet) {
                let cleanSnippet = res.snippet.replace(/<\/?b>/g, '').trim();
                let snippet = new St.Label({
                    text: cleanSnippet.length > 100 ? cleanSnippet.substring(0, 100) + '...' : cleanSnippet,
                    style_class: 'lens-result-snippet',
                });
                textBox.add_child(snippet);
            }

            itemBox.add_child(textBox);
            this._resultsBox.add_child(itemBox);
            this._resultWidgets.push(itemBox);
        }

        if (this._results.length > 0) {
            this._setSelectedIndex(0);
        }
    }

    _setStatus(text) {
        if (!text) {
            this._statusContainer.hide();
            return;
        }
        this._statusLabel.set_text(text);
        this._statusContainer.show();
    }

    _startAnimation(baseText) {
        if (!this._settings.get_boolean('show-llm-animations')) {
            this._setStatus(baseText);
            return;
        }

        this._stopAnimation();
        this._activeStatusText = baseText;
        this._statusContainer.show();

        this._llmTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 400, () => {
            this._llmDotCount = (this._llmDotCount + 1) % 4;
            let dots = '.'.repeat(this._llmDotCount);
            this._statusLabel.set_text(this._activeStatusText + dots);
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stopAnimation() {
        if (this._llmTimerId > 0) {
            GLib.source_remove(this._llmTimerId);
            this._llmTimerId = 0;
        }
        this._statusContainer.hide();
    }

    destroy() {
        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }

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

        this._cancelBackendRequest();
        this._stopAnimation();

        this.remove_all_transitions();
        if (this._dialog) {
            this._dialog.remove_all_transitions();
        }

        for (let widget of this._resultWidgets) {
            if (widget) {
                widget.reactive = false;
                widget.remove_all_transitions();
            }
        }

        this.disconnectObject(this);
        Main.layoutManager.disconnectObject(this);
        super.destroy();
    }
});

const GnomeLensIndicator = GObject.registerClass(
    class GnomeLensIndicator extends PanelMenu.Button {
        _init(extension, settings) {
            super._init(0.0, 'Gnome Lens', false);
            this._extension = extension;
            this._settings = settings;

            let icon = new St.Icon({
                icon_name: 'system-search-symbolic',
                style_class: 'system-status-icon',
            });
            this.add_child(icon);

            this._buildMenu();

            this._settings.connectObject('changed::search-history', this._buildMenu.bind(this), this);
            this._settings.connectObject('changed::enable-history', this._buildMenu.bind(this), this);

            this.connectObject('captured-event', this._onCapturedEvent.bind(this), this);
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

        _buildMenu() {
            this.menu.removeAll();

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

            let prefsItem = new PopupMenu.PopupMenuItem('Preferences');
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

export default class GnomeLensExtension extends Extension {
    enable() {
        this._settings = this.getSettings('org.gnome.shell.extensions.gnome-lens');
        this._ui = null;

        this._indicator = new GnomeLensIndicator(this, this._settings);
        Main.panel.addToStatusArea('gnome-lens', this._indicator);

        this._settings.connectObject('changed::shortcut', this._bindShortcut.bind(this), this);
        this._bindShortcut();
    }

    disable() {
        Main.wm.removeKeybinding('shortcut');

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }

        if (this._ui) {
            this._ui.destroy();
            this._ui = null;
        }

        if (this._settings) {
            this._settings.disconnectObject(this);
            this._settings = null;
        }
    }

    _bindShortcut() {
        Main.wm.removeKeybinding('shortcut');
        Main.wm.addKeybinding(
            'shortcut',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            this.toggleLens.bind(this)
        );
    }

    toggleLens() {
        if (!this._ui) {
            this._ui = new GnomeLensUI(this._settings, this);
        }

        if (this._ui.isOpen) {
            this._ui.close(true);
        } else {
            this._ui.open();
        }
    }

    openLensWithQuery(query) {
        if (!this._ui) {
            this._ui = new GnomeLensUI(this._settings, this);
        }

        if (!this._ui.isOpen) {
            this._ui.open();
        }

        this._ui.setQuery(query);
    }

    saveHistory(query) {
        if (!this._settings.get_boolean('enable-history')) {
            return;
        }
        if (!query || query.trim().length === 0) {
            return;
        }

        query = query.trim();
        let history = this._settings.get_strv('search-history') || [];

        let idx = history.indexOf(query);
        if (idx !== -1) {
            history.splice(idx, 1);
        }

        history.unshift(query);
        if (history.length > 10) {
            history.pop();
        }

        this._settings.set_strv('search-history', history);
    }
}
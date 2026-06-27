// gnome-extension/ui_preview.js
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import { GnomeLensImagePreview } from './ui_preview_image.js';
import { GnomeLensVideoPreview } from './ui_preview_video.js';

const GnomeLensPreview = GObject.registerClass({
    GTypeName: 'GnomeLensPreview'
}, class GnomeLensPreview extends St.Widget {
    _init(settings) {
        super._init({
            reactive: true,
            can_focus: false,
            visible: false,
            clip_to_allocation: true,
            style_class: 'lens-dialog lens-preview-dialog'
        });

        this._settings = settings;
        this._filepath = null;
        this._type = null;
        this._activeStrategy = null;

        let w = this._settings.get_int('preview-w');
        let h = this._settings.get_int('preview-h');
        let x = this._settings.get_int('preview-x');
        let y = this._settings.get_int('preview-y');

        let monitor = Main.layoutManager.primaryMonitor;
        if (x === -1 || y === -1) {
            w = 500; h = 350;
            x = monitor.width - w - 50;
            y = 50;
        }

        this.set_position(x, y);
        this.set_size(w, h);

        this._buildUI();

        this.connectObject('button-press-event', () => {
            return Clutter.EVENT_STOP;
        }, this);
    }

    _buildUI() {
        this._layout = new St.BoxLayout({ vertical: true, x_expand: true, y_expand: true });

        this._header = new St.BoxLayout({
            style_class: 'lens-preview-header',
            reactive: true,
            x_expand: true,
            y_align: Clutter.ActorAlign.START
        });

        let title = new St.Label({ text: 'Media Preview', style_class: 'lens-preview-title', x_expand: true });
        this._header.add_child(title);
        this._layout.add_child(this._header);

        this._contentBox = new St.BoxLayout({
            style_class: 'lens-preview-content',
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL
        });

        this._layout.add_child(this._contentBox);
        
        this._resizeHandle = new St.Widget({
            style_class: 'lens-preview-resize-handle',
            reactive: true,
            width: 20, height: 20
        });

        this.add_child(this._layout);
        this.add_child(this._resizeHandle);

        this.connectObject('notify::width', () => this._updateHandlePos(), this);
        this.connectObject('notify::height', () => this._updateHandlePos(), this);
        this._updateHandlePos();

        let dragging = false;
        let startX, startY, startWinX, startWinY;
        
        this._header.connectObject('button-press-event', (actor, event) => {
            dragging = true;
            let [x, y] = event.get_coords();
            startX = x; startY = y;
            startWinX = this.x; startWinY = this.y;
            return Clutter.EVENT_STOP;
        }, this);
        
        this._header.connectObject('motion-event', (actor, event) => {
            if (!dragging) return Clutter.EVENT_PROPAGATE;
            let [x, y] = event.get_coords();
            this.set_position(startWinX + (x - startX), startWinY + (y - startY));
            return Clutter.EVENT_STOP;
        }, this);
        
        this._header.connectObject('button-release-event', () => {
            dragging = false;
            this._saveGeometry();
            return Clutter.EVENT_STOP;
        }, this);

        let resizing = false;
        let startW, startH;
        this._resizeHandle.connectObject('button-press-event', (actor, event) => {
            resizing = true;
            let [x, y] = event.get_coords();
            startX = x; startY = y;
            startW = this.width; startH = this.height;
            return Clutter.EVENT_STOP;
        }, this);
        
        this._resizeHandle.connectObject('motion-event', (actor, event) => {
            if (!resizing) return Clutter.EVENT_PROPAGATE;
            let [x, y] = event.get_coords();
            let newW = Math.max(300, startW + (x - startX));
            let newH = Math.max(200, startH + (y - startY));
            this.set_size(newW, newH);
            return Clutter.EVENT_STOP;
        }, this);
        
        this._resizeHandle.connectObject('button-release-event', () => {
            resizing = false;
            this._saveGeometry();
            return Clutter.EVENT_STOP;
        }, this);
    }

    _updateHandlePos() {
        this._resizeHandle.set_position(this.width - 20, this.height - 20);
        this._layout.set_size(this.width, this.height);
    }

    _saveGeometry() {
        this._settings.set_int('preview-x', this.x);
        this._settings.set_int('preview-y', this.y);
        this._settings.set_int('preview-w', this.width);
        this._settings.set_int('preview-h', this.height);
    }

    isVisible() { return this.visible; }

    isVideo() { return this._type === 'video'; }

    scrub(offset) {
        if (this.isVideo() && this._activeStrategy && typeof this._activeStrategy.scrub === 'function') {
            this._activeStrategy.scrub(offset);
        }
    }

    showFile(filepath, type) {
        console.log(`[Gnome Lens Debug] showFile called for: ${filepath} [${type}]`);
        if (this._filepath === filepath && this.visible) return;
        this._filepath = filepath;
        this._type = type;

        if (this.get_parent()) {
            let parent = this.get_parent();
            parent.remove_child(this);
            parent.add_child(this);
        }
        
        this.show();

        if (this._activeStrategy) {
            this._activeStrategy.destroy();
            this._activeStrategy = null;
        }

        this._contentBox.destroy_all_children();

        if (type === 'image') {
            this._activeStrategy = new GnomeLensImagePreview(filepath);
        } else if (type === 'video') {
            this._activeStrategy = new GnomeLensVideoPreview(filepath);
        }

        if (this._activeStrategy) {
            this._contentBox.add_child(this._activeStrategy);
        }
    }

    hide() {
        this.visible = false;
        this._filepath = null;
        this._type = null;
        
        if (this._activeStrategy) {
            this._activeStrategy.destroy();
            this._activeStrategy = null;
        }
    }

    destroy() {
        this.hide();
        if (this.get_parent()) {
            this.get_parent().remove_child(this);
        }
        this.disconnectObject(this);
        super.destroy();
    }
});

export { GnomeLensPreview };
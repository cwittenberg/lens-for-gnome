// gnome-extension/ui_preview.js
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import { GnomeLensImagePreview } from './ui_preview_image.js';
import { GnomeLensVideoPreview } from './ui_preview_video.js';

export const GnomeLensPreview = GObject.registerClass({
    GTypeName: 'GnomeLensPreview'
}, class GnomeLensPreview extends St.Widget {
    _init(settings) {
        super._init({
            reactive: true,
            can_focus: true,
            visible: false,
            clip_to_allocation: true,
            style_class: 'lens-dialog lens-preview-dialog'
        });

        this.set_layout_manager(new Clutter.BinLayout());

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

        this._isDragging = false;
        this._dragStartX = 0;
        this._dragStartY = 0;
        this._dragStartWinX = 0;
        this._dragStartWinY = 0;

        this._buildUI();

        this.connectObject('button-press-event', () => Clutter.EVENT_STOP, this);

        this.connectObject('scroll-event', (actor, event) => {
            if (this.isVideo() && this._activeStrategy) {
                let direction = event.get_scroll_direction();
                let delta = 0;
                
                if (direction === Clutter.ScrollDirection.UP) {
                    delta = 5;
                } else if (direction === Clutter.ScrollDirection.DOWN) {
                    delta = -5;
                } else if (direction === Clutter.ScrollDirection.SMOOTH) {
                    // FIX: Clutter.Event does not expose a native get_scroll_deltas() method binding in GJS.
                    // Instead, we use the correct GObject property event.get_scroll_delta() which returns [dx, dy].
                    let [dx, dy] = event.get_scroll_delta();
                    if (dy !== 0) {
                        delta = -dy * 5; 
                    }
                }

                if (delta !== 0) {
                    this._activeStrategy.scrub(delta);
                    return Clutter.EVENT_STOP;
                }
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);
    }

    _buildUI() {
        this._contentBox = new St.Widget({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL
        });
        this._contentBox.set_layout_manager(new Clutter.BinLayout());
        this.add_child(this._contentBox);
        
        this._resizeHandle = new St.Widget({
            style_class: 'lens-preview-resize-handle',
            reactive: true,
            width: 20, height: 20,
            x_expand: true, y_expand: true,
            x_align: Clutter.ActorAlign.END,
            y_align: Clutter.ActorAlign.END
        });
        this.add_child(this._resizeHandle);

        this.isFullscreen = false;
        this._preFsGeom = { x: 0, y: 0, w: 0, h: 0 };

        // Handle dragging directly on the component via safe internal pointer propagation tracking
        this.connectObject(
            'button-press-event', (actor, event) => {
                if (event.get_button() !== 1) return Clutter.EVENT_PROPAGATE;

                let source = event.get_source();
                let isControl = false;
                let current = source;
                while (current && current !== this) {
                    let cls = current.get_style_class_name ? current.get_style_class_name() : '';
                    if (cls && (cls.includes('hud') || cls.includes('resize-handle') || cls.includes('btn') || cls.includes('slider') || cls.includes('track') || cls.includes('fill'))) {
                        isControl = true;
                        break;
                    }
                    current = current.get_parent();
                }

                if (event.get_click_count() === 2 && !isControl) {
                    this.toggleFullscreen();
                    return Clutter.EVENT_STOP;
                }

                if (!isControl && !this.isFullscreen) {
                    let [x, y] = event.get_coords();
                    this._isDragging = true;
                    this._dragStartX = x;
                    this._dragStartY = y;
                    this._dragStartWinX = this.x;
                    this._dragStartWinY = this.y;
                    return Clutter.EVENT_STOP;
                }
                return Clutter.EVENT_PROPAGATE;
            },
            'motion-event', (actor, event) => {
                if (this.isVideo() && this._activeStrategy && typeof this._activeStrategy._resetHideTimer === 'function') {
                    this._activeStrategy._resetHideTimer();
                }

                if (!this._isDragging) return Clutter.EVENT_PROPAGATE;

                let [x, y] = event.get_coords();
                let deltaX = x - this._dragStartX;
                let deltaY = y - this._dragStartY;

                this.set_position(
                    this._dragStartWinX + deltaX,
                    this._dragStartWinY + deltaY
                );
                return Clutter.EVENT_STOP;
            },
            'button-release-event', (actor, event) => {
                if (event.get_button() !== 1) return Clutter.EVENT_PROPAGATE;
                if (this._isDragging) {
                    this._isDragging = false;
                    this._saveGeometry();
                    return Clutter.EVENT_STOP;
                }
                return Clutter.EVENT_PROPAGATE;
            },
            this
        );

        let resizing = false;
        let startX, startY, startW, startH;
        this._resizeHandle.connectObject('button-press-event', (actor, event) => {
            if (this.isFullscreen) return Clutter.EVENT_STOP;
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
            if (resizing) {
                resizing = false;
                this._saveGeometry();
            }
            return Clutter.EVENT_STOP;
        }, this);
    }

    toggleFullscreen() {
        if (this.isFullscreen) {
            this.set_position(this._preFsGeom.x, this._preFsGeom.y);
            this.set_size(this._preFsGeom.w, this._preFsGeom.h);
            this.isFullscreen = false;
        } else {
            this._preFsGeom = { x: this.x, y: this.y, w: this.width, h: this.height };
            let monitor = Main.layoutManager.primaryMonitor;
            this.set_position(monitor.x, monitor.y);
            this.set_size(monitor.width, monitor.height);
            this.isFullscreen = true;
        }
    }

    _saveGeometry() {
        if (this.isFullscreen) return;
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
        if (this._filepath === filepath && this.visible) return;
        
        if (this._activeStrategy && typeof this._activeStrategy.saveCurrentPosition === 'function') {
            this._activeStrategy.saveCurrentPosition();
        }

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
        if (this._activeStrategy && typeof this._activeStrategy.saveCurrentPosition === 'function') {
            this._activeStrategy.saveCurrentPosition();
        }

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
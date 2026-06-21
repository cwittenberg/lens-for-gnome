import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';

export const GnomeLensResultsList = GObject.registerClass(
class GnomeLensResultsList extends St.ScrollView {
    _init(callbacks) {
        super._init({
            style_class: 'lens-results-scroll',
            x_expand: true,
            y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
        });

        this.callbacks = callbacks || {};
        this._results = [];
        this._resultWidgets = [];
        this._selectedIndex = -1;

        this._lastPointerX = -1;
        this._lastPointerY = -1;

        this._resultsBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
        });
        this.add_child(this._resultsBox);
    }

    addSynthesisWidget(widget) {
        this._resultsBox.add_child(widget);
    }

    hasResults() { 
        return this._results.length > 0; 
    }
    
    getCount() { 
        return this._results.length; 
    }
    
    getSelectedIndex() { 
        return this._selectedIndex; 
    }

    selectNext() {
        if (this._resultWidgets.length > 0 && this._selectedIndex < this._resultWidgets.length - 1) {
            this._setSelectedIndex(this._selectedIndex + 1);
        }
    }

    selectPrev() {
        if (this._selectedIndex > 0) {
            this._setSelectedIndex(this._selectedIndex - 1);
        } else if (this._selectedIndex === 0) {
            this._setSelectedIndex(-1);
        }
    }

    launchSelected() {
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            if (this.callbacks.onLaunch) this.callbacks.onLaunch(this._results[this._selectedIndex]);
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

            let adjustment = this.vscroll.adjustment;
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

    clear() {
        this._results = [];
        this._selectedIndex = -1;
        for (let widget of this._resultWidgets) {
            widget.reactive = false;
            widget.remove_all_transitions();
            widget.destroy();
        }
        this._resultWidgets = [];
    }

    _fetchThumbnailAsync(filepath, iconActor, fallbackIconName) {
        let file = Gio.File.new_for_path(filepath);
        let uri = file.get_uri();
        let hash = GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, uri, -1);
        
        let paths = [
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'normal', hash + '.png']),
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'large', hash + '.png'])
        ];

        let checkNext = (index) => {
            if (index >= paths.length) return;
            
            let thumbFile = Gio.File.new_for_path(paths[index]);
            thumbFile.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (f, res) => {
                try {
                    f.query_info_finish(res);
                    iconActor.set_gicon(new Gio.FileIcon({ file: thumbFile }));
                    iconActor.add_style_class_name('lens-result-preview');
                    iconActor.remove_style_class_name('lens-result-icon');
                } catch (e) {
                    checkNext(index + 1);
                }
            });
        };

        checkNext(0);
    }

    renderResults(resultsArray) {
        // Cache the currently selected ID to prevent sticky cursor jumping 
        // when background LLM filtering yields a dynamic refresh.
        let oldSelectedId = null;
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            oldSelectedId = this._results[this._selectedIndex].id;
        }

        this.clear();
        this._results = resultsArray;

        let maxRender = Math.min(resultsArray.length, 30);

        for (let i = 0; i < maxRender; i++) {
            let res = resultsArray[i];

            let itemBox = new St.BoxLayout({
                style_class: 'lens-result-item',
                vertical: false,
                reactive: true,
            });

            itemBox.connectObject('button-press-event', () => {
                if (this.callbacks.onLaunch) this.callbacks.onLaunch(res);
                return Clutter.EVENT_STOP;
            }, this);

            let handlePointerEvent = () => {
                let [x, y] = global.get_pointer();
                if (this._lastPointerX !== x || this._lastPointerY !== y) {
                    this._lastPointerX = x;
                    this._lastPointerY = y;
                    if (this._selectedIndex !== i) {
                        this._setSelectedIndex(i);
                    }
                }
                return Clutter.EVENT_PROPAGATE;
            };

            itemBox.connectObject('enter-event', handlePointerEvent, this);
            itemBox.connectObject('motion-event', handlePointerEvent, this);

            let isImagePreview = false;
            let isVideoPreview = false;
            let iconName = 'text-x-generic-symbolic';
            
            if (res.metadata && res.metadata.filetype && res.filepath) {
                let ext = res.metadata.filetype.toLowerCase();
                if (['png', 'jpg', 'jpeg', 'bmp', 'webp', 'svg'].includes(ext)) {
                    isImagePreview = true;
                    iconName = 'image-x-generic-symbolic';
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

            let iconActor = new St.Icon({
                icon_name: iconName,
                style_class: 'lens-result-icon',
            });

            if ((isImagePreview || isVideoPreview) && res.filepath) {
                this._fetchThumbnailAsync(res.filepath, iconActor, iconName);
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
            let newIndex = 0;
            if (oldSelectedId) {
                let found = this._results.findIndex(r => r.id === oldSelectedId);
                if (found !== -1 && found < this._resultWidgets.length) {
                    newIndex = found;
                }
            }
            this._setSelectedIndex(newIndex);
        }
    }

    destroy() {
        this.clear();
        this.disconnectObject(this);
        super.destroy();
    }
});
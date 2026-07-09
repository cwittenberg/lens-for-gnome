import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const GnomeLensPdfPreview = GObject.registerClass({
    GTypeName: 'GnomeLensPdfPreview'
}, class GnomeLensPdfPreview extends St.BoxLayout {
    _init(filepath) {
        super._init({
            vertical: true,
            x_expand: true,
            y_expand: true,
            style: 'background-color: #1a1a1a;'
        });
        
        this._filepath = filepath;
        this._prefixId = GLib.uuid_string_random();
        this._tempPrefix = GLib.build_filenamev([GLib.get_tmp_dir(), `lens-pdf-${this._prefixId}`]);
        this._tempFiles = [];
        this._proc = null;
        this._cancellable = new Gio.Cancellable();
        
        this._buildHeader();

        this._scroll = new St.ScrollView({
            x_expand: true,
            y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            style: 'padding: 16px;'
        });

        this._pagesBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.CENTER,
        });

        this._loadingLabel = new St.Label({
            text: 'Generating PDF preview pages...',
            style: 'color: rgba(255,255,255,0.6); font-size: 11pt;',
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
            margin_top: 40
        });
        this._pagesBox.add_child(this._loadingLabel);

        this._scroll.add_child(this._pagesBox);
        this.add_child(this._scroll);

        this.connectObject('destroy', () => this._onDestroy(), this);
        this._extractPreview();
    }

    _buildHeader() {
        let header = new St.BoxLayout({
            vertical: false,
            style: 'background-color: rgba(0, 0, 0, 0.4); padding: 8px 12px; border-bottom: 1px solid rgba(255, 255, 255, 0.1);',
            y_align: Clutter.ActorAlign.CENTER
        });

        let title = new St.Label({
            text: GLib.path_get_basename(this._filepath),
            y_align: Clutter.ActorAlign.CENTER,
            x_expand: true,
            style: 'color: #ffffff; font-weight: bold; font-size: 11pt;'
        });
        header.add_child(title);

        let openBtn = new St.Button({
            child: new St.Icon({ icon_name: 'document-open-symbolic', icon_size: 16 }),
            style: 'background-color: rgba(255, 255, 255, 0.15); border-radius: 4px; padding: 6px;',
            y_align: Clutter.ActorAlign.CENTER
        });
        
        openBtn.connectObject('clicked', () => {
            let file = Gio.File.new_for_path(this._filepath);
            Gio.AppInfo.launch_default_for_uri_async(file.get_uri(), null, null, null);
        }, this);
        header.add_child(openBtn);

        this.add_child(header);
    }

    _extractPreview() {
        let cmd = ['pdftocairo', '-jpeg', '-scale-to', '1000', '-l', '15', this._filepath, this._tempPrefix];
        
        try {
            this._proc = Gio.Subprocess.new(cmd, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            this._proc.wait_async(this._cancellable, (p, res) => {
                try {
                    this._proc.wait_finish(res);
                    this._proc = null;
                } catch (e) { 
                    return; 
                }

                this._loadRenderedPages();
            });
        } catch (e) {
            if (this._loadingLabel) {
                this._loadingLabel.set_text('Preview unavailable (pdftocairo missing or failed)');
            }
        }
    }

    _loadRenderedPages() {
        if (this._cancellable.is_cancelled()) return;

        let tmpDir = Gio.File.new_for_path(GLib.get_tmp_dir());
        tmpDir.enumerate_children_async('standard::name', Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, this._cancellable, (obj, res) => {
            try {
                let iter = obj.enumerate_children_finish(res);
                let files = [];
                
                let nextBatch = () => {
                    iter.next_files_async(50, GLib.PRIORITY_DEFAULT, this._cancellable, (it, queryRes) => {
                        try {
                            let batch = it.next_files_finish(queryRes);
                            if (batch && batch.length > 0) {
                                for (let info of batch) {
                                    let name = info.get_name();
                                    if (name.startsWith(`lens-pdf-${this._prefixId}`) && name.endsWith('.jpg')) {
                                        files.push(name);
                                    }
                                }
                                nextBatch();
                            } else {
                                it.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                                this._displayPages(files);
                            }
                        } catch (e) {
                            it.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                            this._displayPages(files);
                        }
                    });
                };
                nextBatch();
            } catch (e) {
                if (this._loadingLabel) {
                    this._loadingLabel.set_text('Failed to load generated pages.');
                }
            }
        });
    }

    _displayPages(files) {
        if (this._cancellable.is_cancelled()) return;
        
        if (files.length === 0) {
            if (this._loadingLabel) {
                this._loadingLabel.set_text('No pages generated. Document might be empty or corrupted.');
            }
            return;
        }

        if (this._loadingLabel) {
            this._loadingLabel.destroy();
            this._loadingLabel = null;
        }

        files.sort();

        for (let name of files) {
            let pageFile = GLib.build_filenamev([GLib.get_tmp_dir(), name]);
            this._tempFiles.push(pageFile);

            // Reverted back to hardcoded dimensions which are strictly reliable and 
            // do not cause the widget initialization to fail/crash before rendering.
            let pageWidget = new St.Widget({
                style: `background-image: url("file://${pageFile}"); background-size: contain; background-repeat: no-repeat; background-position: center; background-color: #ffffff; border-radius: 4px; border: 1px solid rgba(255,255,255,0.1); margin-bottom: 24px;`,
                width: 700,
                height: 990 
            });
            
            this._pagesBox.add_child(pageWidget);
        }
        
        if (files.length === 15) {
            let notice = new St.Label({
                text: 'Preview limited to the first 15 pages.',
                style: 'color: rgba(255,255,255,0.4); font-size: 10pt; font-style: italic;',
                margin_bottom: 24
            });
            this._pagesBox.add_child(notice);
        }
    }

    _onDestroy() {
        this._cancellable.cancel();
        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }
        for (let f of this._tempFiles) {
            let file = Gio.File.new_for_path(f);
            file.delete_async(GLib.PRIORITY_DEFAULT, null, (df, dres) => {
                try { df.delete_finish(dres); } catch(e) {}
            });
        }
    }
});
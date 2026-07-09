import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const GnomeLensOfficePreview = GObject.registerClass({
    GTypeName: 'GnomeLensOfficePreview'
}, class GnomeLensOfficePreview extends St.BoxLayout {
    _init(filepath) {
        super._init({
            vertical: true,
            x_expand: true,
            y_expand: true,
            style: 'background-color: #1a1a1a;'
        });
        
        this._filepath = filepath;
        this._prefixId = GLib.uuid_string_random();
        this._workDir = GLib.build_filenamev([GLib.get_tmp_dir(), `lens-office-${this._prefixId}`]);
        this._tempPrefix = GLib.build_filenamev([this._workDir, 'page']);
        
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
            text: 'Starting Office Bridge...',
            style: 'color: rgba(255,255,255,0.6); font-size: 11pt;',
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
            margin_top: 40
        });
        this._pagesBox.add_child(this._loadingLabel);

        this._scroll.add_child(this._pagesBox);
        this.add_child(this._scroll);

        this.connectObject('destroy', () => this._onDestroy(), this);
        this._initWorkDirAndConvert();
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
            child: new St.Icon({ icon_name: 'external-link-symbolic', icon_size: 16 }),
            style_class: 'lens-result-action-btn',
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            can_focus: true
        });
        
        openBtn.connectObject('button-press-event', () => {
            let file = Gio.File.new_for_path(this._filepath);
            Gio.AppInfo.launch_default_for_uri_async(file.get_uri(), null, null, null);
            return Clutter.EVENT_STOP;
        }, this);
        header.add_child(openBtn);

        this.add_child(header);
    }

    _initWorkDirAndConvert() {
        let dir = Gio.File.new_for_path(this._workDir);
        dir.make_directory_async(GLib.PRIORITY_DEFAULT, this._cancellable, (f, res) => {
            try {
                f.make_directory_finish(res);
                this._convertDocument();
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED)) {
                    this._showError('Failed to create workspace directory.');
                }
            }
        });
    }

    _showError(msg) {
        if (this._loadingLabel && !this._cancellable.is_cancelled()) {
            this._loadingLabel.set_text(msg);
        }
    }

    _convertDocument() {
        this._loadingLabel.set_text('Converting Document... (This may take a moment for large files)');
        
        let exec = GLib.find_program_in_path('libreoffice') ? 'libreoffice' : 'soffice';
        let cmd = [exec, '--headless', '--nologo', '--nofirststartwizard', '--convert-to', 'pdf', '--outdir', this._workDir, this._filepath];
        
        try {
            this._proc = Gio.Subprocess.new(cmd, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            this._proc.wait_async(this._cancellable, (p, res) => {
                try {
                    this._proc.wait_finish(res);
                    this._proc = null;
                } catch (e) { 
                    return; 
                }

                this._findConvertedPdf();
            });
        } catch (e) {
            this._showError('Preview unavailable (LibreOffice is required for Office documents but is not installed)');
        }
    }

    _findConvertedPdf() {
        if (this._cancellable.is_cancelled()) return;

        let dir = Gio.File.new_for_path(this._workDir);
        dir.enumerate_children_async('standard::name', Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, this._cancellable, (obj, res) => {
            try {
                let iter = obj.enumerate_children_finish(res);
                let nextBatch = () => {
                    iter.next_files_async(10, GLib.PRIORITY_DEFAULT, this._cancellable, (it, queryRes) => {
                        try {
                            let batch = it.next_files_finish(queryRes);
                            if (batch && batch.length > 0) {
                                for (let info of batch) {
                                    if (info.get_name().endsWith('.pdf')) {
                                        let pdfPath = GLib.build_filenamev([this._workDir, info.get_name()]);
                                        this._tempFiles.push(pdfPath);
                                        this._extractJpgs(pdfPath);
                                        it.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                                        return;
                                    }
                                }
                                nextBatch();
                            } else {
                                it.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                                this._showError('Conversion failed: No PDF generated.');
                            }
                        } catch (e) {
                            it.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                            this._showError('Error reading converted directory.');
                        }
                    });
                };
                nextBatch();
            } catch (e) {
                this._showError('Error opening converted directory.');
            }
        });
    }

    _extractJpgs(pdfPath) {
        this._loadingLabel.set_text('Rendering pages...');
        let cmd = ['pdftocairo', '-jpeg', '-scale-to', '1000', '-l', '15', pdfPath, this._tempPrefix];
        
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
            this._showError('Preview unavailable (pdftocairo missing or failed)');
        }
    }

    _loadRenderedPages() {
        if (this._cancellable.is_cancelled()) return;

        let dir = Gio.File.new_for_path(this._workDir);
        dir.enumerate_children_async('standard::name', Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, this._cancellable, (obj, res) => {
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
                                    if (name.startsWith('page') && name.endsWith('.jpg')) {
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
                this._showError('Failed to load generated pages.');
            }
        });
    }

    _displayPages(files) {
        if (this._cancellable.is_cancelled()) return;
        
        if (files.length === 0) {
            this._showError('No pages generated. Document might be empty.');
            return;
        }

        if (this._loadingLabel) {
            this._loadingLabel.destroy();
            this._loadingLabel = null;
        }

        files.sort();

        for (let name of files) {
            let pageFile = GLib.build_filenamev([this._workDir, name]);
            this._tempFiles.push(pageFile);

            let pageWidget = new St.Widget({
                style: `background-image: url("file://${pageFile}"); background-size: contain; background-repeat: no-repeat; background-position: center; background-color: #ffffff; border-radius: 4px; border: 1px solid rgba(255,255,255,0.1); margin-bottom: 24px;`,
                width: 700,
                height: 990 
            });
            
            this._pagesBox.add_child(pageWidget);
        }
        
        if (files.length === 15) {
            let notice = new St.Label({
                text: 'Preview limited to the first 15 pages for performance.',
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

        let cleanupList = [...this._tempFiles];
        let workDir = this._workDir;

        let deleteNext = (index) => {
            if (index >= cleanupList.length) {
                let dirFile = Gio.File.new_for_path(workDir);
                dirFile.delete_async(GLib.PRIORITY_DEFAULT, null, (df, dres) => {
                    try { df.delete_finish(dres); } catch(e) {}
                });
                return;
            }
            
            let f = Gio.File.new_for_path(cleanupList[index]);
            f.delete_async(GLib.PRIORITY_DEFAULT, null, (df, dres) => {
                try { df.delete_finish(dres); } catch(e) {}
                deleteNext(index + 1);
            });
        };
        
        deleteNext(0);
    }
});
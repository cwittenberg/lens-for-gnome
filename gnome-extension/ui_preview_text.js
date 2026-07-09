import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export const GnomeLensTextPreview = GObject.registerClass({
    GTypeName: 'GnomeLensTextPreview'
}, class GnomeLensTextPreview extends St.BoxLayout {
    _init(filepath) {
        super._init({
            vertical: true,
            x_expand: true,
            y_expand: true,
            style: 'background-color: #1e1e1e;'
        });

        this._filepath = filepath;
        this._cancellable = new Gio.Cancellable();
        this._copyTimeoutId = 0;
        this._textContent = '';

        this._buildHeader();

        // Main viewer container holding numbers and code columns side-by-side
        this._scroll = new St.ScrollView({
            x_expand: true,
            y_expand: true,
            hscrollbar_policy: St.PolicyType.AUTOMATIC,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            style: 'padding: 8px;'
        });

        this._workspaceBox = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_expand: true
        });

        // Left column for line numbers
        this._lineNumbersLabel = new St.Label({
            style: 'font-family: monospace; color: #5c6370; font-size: 10pt; text-align: right; padding-right: 12px; border-right: 1px solid rgba(255, 255, 255, 0.1);'
        });
        this._lineNumbersLabel.clutter_text.line_wrap = false;
        this._lineNumbersLabel.clutter_text.single_line_mode = false;
        this._workspaceBox.add_child(this._lineNumbersLabel);

        // Right column for the highlighted code contents
        this._codeLabel = new St.Label({
            style: 'font-family: monospace; color: #abb2bf; font-size: 10pt; padding-left: 12px;'
        });
        this._codeLabel.clutter_text.line_wrap = false;
        this._codeLabel.clutter_text.single_line_mode = false;
        this._codeLabel.clutter_text.use_markup = true; // Enables Pango syntax highlighting tags
        this._workspaceBox.add_child(this._codeLabel);

        this._scroll.add_child(this._workspaceBox);
        this.add_child(this._scroll);

        this.connectObject('destroy', () => {
            this._cancellable.cancel();
            if (this._copyTimeoutId) {
                GLib.source_remove(this._copyTimeoutId);
                this._copyTimeoutId = 0;
            }
        }, this);

        this._loadFile();
    }

    _buildHeader() {
        let header = new St.BoxLayout({
            vertical: false,
            style: 'background-color: rgba(0, 0, 0, 0.3); padding: 8px 12px; border-bottom: 1px solid rgba(255, 255, 255, 0.08);',
            y_align: Clutter.ActorAlign.CENTER
        });

        let title = new St.Label({
            text: GLib.path_get_basename(this._filepath),
            y_align: Clutter.ActorAlign.CENTER,
            x_expand: true,
            style: 'color: #ffffff; font-weight: bold; font-size: 11pt;'
        });
        header.add_child(title);

        let copyBtn = new St.Button({
            child: new St.Icon({ icon_name: 'edit-copy-symbolic', icon_size: 16 }),
            style_class: 'lens-video-control-btn', 
            y_align: Clutter.ActorAlign.CENTER
        });
        
        copyBtn.connectObject('clicked', () => {
            if (this._textContent) {
                let clipboard = St.Clipboard.get_default();
                clipboard.set_text(St.ClipboardType.CLIPBOARD, this._textContent);
                
                copyBtn.set_child(new St.Icon({ icon_name: 'emblem-ok-symbolic', icon_size: 16 }));
                
                if (this._copyTimeoutId) {
                    GLib.source_remove(this._copyTimeoutId);
                }
                
                this._copyTimeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
                    this._copyTimeoutId = 0;
                    if (copyBtn) {
                        copyBtn.set_child(new St.Icon({ icon_name: 'edit-copy-symbolic', icon_size: 16 }));
                    }
                    return GLib.SOURCE_REMOVE;
                });
            }
        }, this);
        
        header.add_child(copyBtn);
        this.add_child(header);
    }

    _escapeHtml(str) {
        return str.replace(/&/g, '&amp;')
                  .replace(/</g, '&lt;')
                  .replace(/>/g, '&gt;')
                  .replace(/"/g, '&quot;')
                  .replace(/'/g, '&#039;');
    }

    _highlightLine(line, ext) {
        let escaped = this._escapeHtml(line);
        if (!ext) return escaped;

        // Structured regex lexing mapping to Pango-compliant color span annotations
        if (['js', 'ts', 'json', 'runtime'].includes(ext)) {
            escaped = escaped.replace(/\b(const|let|var|function|return|import|export|from|class|extends|new|if|else|for|while|switch|case|break|continue|default|true|false|null|undefined|this|super|async|await)\b/g, '<span foreground="#c678dd">$1</span>');
            escaped = escaped.replace(/(\/\/.*)/g, '<span foreground="#5c6370">$1</span>');
            escaped = escaped.replace(/(&quot;.*?&quot;|&#039;.*?&#039;|`.*?`)/g, '<span foreground="#98c379">$1</span>');
        } else if (['css'].includes(ext)) {
            escaped = escaped.replace(/([A-Za-z0-9_\-\.]+)\s*\{/g, '<span foreground="#61afef">$1</span> {');
            escaped = escaped.replace(/([A-Za-z0-9\-]+)\s*:/g, '<span foreground="#e06c75">$1</span>:');
            escaped = escaped.replace(/(:.*?;)/g, '<span foreground="#d19a66">$1</span>');
        } else if (['xml', 'html', 'svg', 'gschema'].includes(ext)) {
            escaped = escaped.replace(/(&lt;\/?[A-Za-z0-9\-_:]+)/g, '<span foreground="#e06c75">$1</span>');
            escaped = escaped.replace(/(\s[A-Za-z0-9\-_:]+=)/g, '<span foreground="#d19a66">$1</span>');
            escaped = escaped.replace(/(&quot;.*?&quot;|&#039;.*?&#039;)/g, '<span foreground="#98c379">$1</span>');
        } else if (['sh', 'bash', 'py', 'yml', 'yaml'].includes(ext)) {
            escaped = escaped.replace(/\b(def|import|from|return|if|elif|else|for|while|in|break|continue|print|echo|set|exit|then|fi|case|esac)\b/g, '<span foreground="#c678dd">$1</span>');
            escaped = escaped.replace(/(#.*)/g, '<span foreground="#5c6370">$1</span>');
            escaped = escaped.replace(/(&quot;.*?&quot;|&#039;.*?&#039;)/g, '<span foreground="#98c379">$1</span>');
        }
        return escaped;
    }

    _loadFile() {
        let file = Gio.File.new_for_path(this._filepath);
        file.load_contents_async(this._cancellable, (obj, res) => {
            try {
                let [success, contents] = obj.load_contents_finish(res);
                if (success) {
                    let decoder = new TextDecoder('utf-8');
                    let rawText = decoder.decode(contents);
                    
                    if (rawText.length > 150000) {
                        rawText = rawText.substring(0, 150000) + '\n\n... [File truncated for preview safety limit]';
                    }
                    
                    this._textContent = rawText;

                    let ext = this._filepath.split('.').pop().toLowerCase();
                    let lines = rawText.split(/\r?\n/);
                    
                    let lineNumbersContent = '';
                    let codeContent = '';
                    
                    for (let i = 0; i < lines.length; i++) {
                        lineNumbersContent += `${i + 1}\n`;
                        codeContent += `${this._highlightLine(lines[i], ext)}\n`;
                    }

                    this._lineNumbersLabel.set_text(lineNumbersContent);
                    this._codeLabel.clutter_text.set_markup(codeContent);
                }
            } catch (e) {
                if (!e.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED)) {
                    this._codeLabel.set_text(`Error reading target asset template: ${e.message}`);
                }
            }
        });
    }
});
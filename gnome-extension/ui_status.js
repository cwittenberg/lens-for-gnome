import GLib from 'gi://GLib';
import St from 'gi://St';
import GObject from 'gi://GObject';

export const GnomeLensSynthesis = GObject.registerClass(
class GnomeLensSynthesis extends St.BoxLayout {
    _init() {
        super._init({ vertical: true, x_expand: true, visible: false });
        this._synthesisLabel = new St.Label({
            style_class: 'lens-synthesis-text',
            x_expand: true,
        });
        this._synthesisLabel.clutter_text.line_wrap = true;
        this.add_child(this._synthesisLabel);
    }

    setSynthesis(text) {
        if (!text) {
            this.hide();
            this._synthesisLabel.set_text('');
            return;
        }
        this._synthesisLabel.set_text(text);
        this.show();
    }
});

export const GnomeLensStatus = GObject.registerClass(
class GnomeLensStatus extends St.BoxLayout {
    _init(settings) {
        super._init({ style_class: 'lens-status-container', visible: false });
        this._settings = settings;
        this._llmTimerId = 0;
        this._llmDotCount = 0;
        this._activeStatusText = '';

        this._statusLabel = new St.Label({ style_class: 'lens-status-label', text: '' });
        this.add_child(this._statusLabel);
    }

    setStatus(text) {
        if (!text) {
            this.hide();
            return;
        }
        this._statusLabel.set_text(text);
        this.show();
    }

    startAnimation(baseText) {
        if (!this._settings.get_boolean('show-llm-animations')) {
            this.setStatus(baseText);
            return;
        }

        this.stopAnimation();
        this._activeStatusText = baseText;
        this.show();

        this._llmTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 400, () => {
            this._llmDotCount = (this._llmDotCount + 1) % 4;
            let dots = '.'.repeat(this._llmDotCount);
            this._statusLabel.set_text(this._activeStatusText + dots);
            return GLib.SOURCE_CONTINUE;
        });
    }

    stopAnimation() {
        if (this._llmTimerId > 0) {
            GLib.source_remove(this._llmTimerId);
            this._llmTimerId = 0;
        }
        this.hide();
    }

    destroy() {
        this.stopAnimation();
        this.disconnectObject(this);
        super.destroy();
    }
});
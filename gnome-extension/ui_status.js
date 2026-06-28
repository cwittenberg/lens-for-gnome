import GLib from 'gi://GLib';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';

export const GnomeLensSynthesis = GObject.registerClass(
class GnomeLensSynthesis extends St.BoxLayout {
    _init() {
        super._init({ vertical: true, x_expand: true, visible: false, style_class: 'lens-synthesis-container' });
        
        this._headerBox = new St.BoxLayout({ vertical: false, x_expand: true });
        
        this._confidenceBadge = new St.Label({
            style_class: 'lens-synthesis-badge',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._headerBox.add_child(this._confidenceBadge);
        this.add_child(this._headerBox);

        this._answerLabel = new St.Label({
            style_class: 'lens-synthesis-answer',
            x_expand: true,
        });
        this._answerLabel.clutter_text.single_line_mode = false;
        this._answerLabel.clutter_text.line_wrap = true;
        this.add_child(this._answerLabel);
        
        this._evidenceLabel = new St.Label({
            style_class: 'lens-synthesis-reasoning',
            x_expand: true,
        });
        this._evidenceLabel.clutter_text.single_line_mode = false;
        this._evidenceLabel.clutter_text.line_wrap = true;
        this.add_child(this._evidenceLabel);

        this._reasoningLabel = new St.Label({
            style_class: 'lens-synthesis-reasoning',
            x_expand: true,
        });
        this._reasoningLabel.clutter_text.single_line_mode = false;
        this._reasoningLabel.clutter_text.line_wrap = true;
        this.add_child(this._reasoningLabel);
    }

    setSynthesis(resultObj) {
        if (!resultObj) {
            this.hide();
            return;
        }
        
        if (typeof resultObj === 'string') {
            this._answerLabel.set_text(resultObj);
            this._confidenceBadge.hide();
            this._evidenceLabel.hide();
            this._reasoningLabel.hide();
        } else {
            this._answerLabel.set_text(resultObj.answer || '');
            
            if (resultObj.confidence_score !== undefined) {
                let text = `Confidence: ${resultObj.confidence_score}%`;
                if (resultObj.confidence_justification) {
                    text += ` - ${resultObj.confidence_justification}`;
                }
                this._confidenceBadge.set_text(text);
                this._confidenceBadge.show();
            } else {
                this._confidenceBadge.hide();
            }

            if (resultObj.reasoning) {
                let reasoningText = resultObj.reasoning;
                
                if (reasoningText.includes('Extracted Evidence:') && reasoningText.includes('Deduction:')) {
                    let parts = reasoningText.split('Deduction:');
                    this._evidenceLabel.set_text(parts[0].trim());
                    this._evidenceLabel.show();
                    this._reasoningLabel.set_text(`Deduction: ${parts[1].trim()}`);
                    this._reasoningLabel.show();
                } else {
                    this._evidenceLabel.hide();
                    this._reasoningLabel.set_text(`Reasoning: ${reasoningText}`);
                    this._reasoningLabel.show();
                }
            } else {
                this._evidenceLabel.hide();
                this._reasoningLabel.hide();
            }
        }
        
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
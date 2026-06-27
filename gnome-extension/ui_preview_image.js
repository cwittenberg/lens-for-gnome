// gnome-extension/ui_preview_image.js
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';

const GnomeLensImagePreview = GObject.registerClass({
    GTypeName: 'GnomeLensImagePreview'
}, class GnomeLensImagePreview extends St.Widget {
    _init(filepath) {
        super._init({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL,
            style: `background-image: url("file://${filepath}"); background-size: contain; background-repeat: no-repeat; background-position: center;`
        });
        console.log(`[Gnome Lens Debug] GnomeLensImagePreview initialized for ${filepath}`);
    }
});

export { GnomeLensImagePreview };
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';

const GnomeLensImagePreview = GObject.registerClass({
    GTypeName: 'GnomeLensImagePreview'
}, class GnomeLensImagePreview extends St.Widget {
    _init(filepath) {
        let file = Gio.File.new_for_path(filepath);
        let uri = file.get_uri();
        
        super._init({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL,
            style: `background-image: url("${uri}"); background-size: contain; background-repeat: no-repeat;`
        });
        console.log(`[Lens for GNOME Debug] GnomeLensImagePreview initialized for ${filepath}`);
    }
});

export { GnomeLensImagePreview };
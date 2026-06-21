// gnome-extension/prefs.js
import { ExtensionPreferences } from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

import { buildGeneralPage } from './prefs_main.js';
import { buildAIPage } from './prefs_ai.js';
import { buildIndexPage } from './prefs_index.js';
import { buildLookAndFeelPage } from './prefs_look_and_feel.js';
import { buildAboutPage } from './prefs_about.js';

export default class GnomeLensPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings('org.gnome.shell.extensions.gnome-lens');
        
        window.add(buildGeneralPage(settings, window));
        window.add(buildLookAndFeelPage(settings, window));
        window.add(buildAIPage(settings, window));
        window.add(buildIndexPage(settings, window));
        window.add(buildAboutPage());
    }
}
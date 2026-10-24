'use strict';

// Rowel: the agent-mode switch, and the single screen the popup shows while it
// is on (spec/05-delivery-modes.md §2). Loaded by popup.html after popup.js.
// The setting is `agentMode` in the extension's settings (background/page.js);
// a body class drives the layout (popup.css), so upstream's statusResponse can
// keep showing and hiding its own blocks underneath. The options page stays
// reachable through the browser's extension menu.
(async () => {
    const AGENT_MODE_CLASS = 'rowel-agent-mode-on';

    const render = function(on) {
        document.body.classList.toggle(AGENT_MODE_CLASS, on);
        $('#agent-mode-switch').checked = on;
    };

    const setAgentMode = async function(on) {
        const settings = await browser.runtime.sendMessage({ action: 'load_settings' });
        settings.agentMode = on;
        await browser.runtime.sendMessage({ action: 'save_settings', args: settings });
        render(on);
    };

    $('#agent-mode-switch').addEventListener('change', (e) => setAgentMode(e.target.checked));
    $('#agent-mode-off-button').addEventListener('click', () => setAgentMode(false));

    const settings = await browser.runtime.sendMessage({ action: 'load_settings' });
    render(settings?.agentMode === true);
})();

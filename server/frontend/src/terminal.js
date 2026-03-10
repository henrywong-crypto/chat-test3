import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';

const config = document.getElementById('app-config').dataset;
const vmId = config.vmId;

const term = new Terminal({ cursorBlink: true, theme: { background: '#000000' } });
const fitAddon = new FitAddon();
term.loadAddon(fitAddon);

const container = document.getElementById('term-container');
term.open(container);
fitAddon.fit();
term.focus();

const ws = new WebSocket(
  (location.protocol === 'https:' ? 'wss:' : 'ws:') + '//' + location.host + '/ws/' + vmId
);
ws.binaryType = 'arraybuffer';

function sendResize() {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: 'resize', rows: term.rows, cols: term.cols }));
  }
}

term.onResize(sendResize);
ws.onopen = () => { term.onData(d => ws.send(new TextEncoder().encode(d))); sendResize(); };
ws.onmessage = e => term.write(new Uint8Array(e.data));
ws.onclose = () => term.write('\r\n\x1b[2mconnection closed\x1b[0m\r\n');
new ResizeObserver(() => fitAddon.fit()).observe(container);

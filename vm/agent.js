import { query } from '@anthropic-ai/claude-agent-sdk';
import readline from 'readline';

const rl = readline.createInterface({ input: process.stdin });
let resolveToolResponse = null;

rl.on('line', line => {
  const msg = JSON.parse(line);
  if (msg.type === 'query') {
    runQuery(msg.content, msg.session_id);
  } else if (msg.type === 'tool-response') {
    resolveToolResponse?.(msg);
  }
});

async function runQuery(content, sessionId) {
  const options = {
    cwd: process.env.HOME,
    ...(sessionId ? { resume: sessionId } : {}),
    canUseTool: async (toolName, input) => {
      send({ type: 'tool-use', tool_name: toolName, input });
      const decision = await new Promise(r => { resolveToolResponse = r; });
      resolveToolResponse = null;
      if (!decision.allow) return { behavior: 'deny' };
      return { behavior: 'allow', updatedInput: decision.updated_input ?? input };
    },
  };

  try {
    for await (const event of query({ prompt: content, options })) {
      if (event.type === 'system' && event.subtype === 'init') {
        send({ type: 'session-started', session_id: event.session_id });
      } else if (event.type === 'assistant') {
        for (const block of event.message.content) {
          if (block.type === 'text') send({ type: 'text', content: block.text });
        }
      } else if (event.type === 'tool') {
        const text = event.content?.flatMap(c => c.content ?? [])
          .filter(c => c.type === 'text').map(c => c.text).join('') ?? '';
        send({ type: 'tool-result', output: text, is_error: !!event.is_error });
      } else if (event.type === 'result') {
        send({ type: 'complete', session_id: event.session_id });
      }
    }
  } catch (err) {
    send({ type: 'error', message: String(err) });
  }
}

function send(obj) { process.stdout.write(JSON.stringify(obj) + '\n'); }

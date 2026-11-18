import os from 'node:os';
import path from 'node:path';
export const SOCK_DIR = path.join(os.homedir(), '.telex', 'claude-channel');
export const sockPath = (id) => path.join(SOCK_DIR, `${id}.sock`);

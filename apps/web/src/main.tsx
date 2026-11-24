import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import { isMacDesktopShell } from './lib/desktopShell';
import { quietScrollbars } from './lib/scrollbars';
import './design/tokens.css';
import './design/global.css';
// The window material only shows through when the page background is clear.
if (isMacDesktopShell()) document.documentElement.dataset.desktop = 'true';
quietScrollbars();
document.addEventListener('contextmenu', (event) => event.preventDefault());
const root = document.getElementById('root');
if (!root) throw new Error('Missing application root');
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// A component in its own module: imports the libraries by name through the page's import map.
import { h } from 'preact';
import htm from 'htm';
export const html = htm.bind(h);
export function Cell({ name, value }) {
  return html`<div class=${'cell' + (value < 5 ? ' low' : '')} id=${'res-' + name}>${name}: ${value}</div>`;
}

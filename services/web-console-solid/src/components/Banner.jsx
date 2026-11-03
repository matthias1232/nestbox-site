// A one-line notice about the state the page is in.
//
// The tone is the whole point: a warning that reads like an info line is a warning
// nobody acts on, so the left border carries the colour of the severity.

import '../styles/controls.css';

export default function Banner(props) {
  return (
    <div class={'nb-banner ' + (props.tone || 'info')} role="status">
      <div class="nb-banner-text">{props.children}</div>
      {props.actions && <div class="nb-banner-actions">{props.actions}</div>}
    </div>
  );
}

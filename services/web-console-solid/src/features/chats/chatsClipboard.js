// Copying masked text, from a button.
//
// `navigator.clipboard` needs a secure context; the console is also opened over
// plain HTTP on a lab network, where the write is refused. The textarea fallback
// is what makes the button work in both places.

/**
 * Copy a text and report whether it worked.
 *
 * The caller shows "kopiert" for a moment and says nothing when it failed,
 * because a clipboard refusal is not a page error.
 */
export async function copyText(text) {
  const value = String(text ?? '');
  if (!value) return false;
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fall through to the selection copy.
  }
  if (typeof document === 'undefined') return false;
  const area = document.createElement('textarea');
  area.value = value;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.append(area);
  area.select();
  let copied = false;
  try {
    copied = document.execCommand('copy');
  } catch {
    copied = false;
  }
  area.remove();
  return copied;
}

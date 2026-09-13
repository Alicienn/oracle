/**
 * A minimal DOM builder.
 *
 * Oracle's UI is small and its updates are targeted, so a virtual DOM would be weight
 * without benefit. `h` is the whole abstraction: it creates an element, applies attributes,
 * and appends children, with enough type safety that a typo in a tag name is a compile error.
 */

type Child = Node | string | number | false | null | undefined;

interface Props {
  class?: string;
  style?: string | Partial<CSSStyleDeclaration>;
  html?: string;
  dataset?: Record<string, string | number | boolean | undefined>;
  [key: string]:
    | string
    | number
    | boolean
    | undefined
    | EventListener
    | Partial<CSSStyleDeclaration>
    | Record<string, string | number | boolean | undefined>;
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props?: Props | null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag);
  applyProps(element, props);
  append(element, children);
  return element;
}

/** SVG needs its own namespace or the browser renders an inert unknown element. */
export function svg(markup: string): SVGElement {
  const wrapper = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  wrapper.innerHTML = markup;
  return wrapper;
}

/** Parses a full SVG string into a live node. Used for the icon set. */
export function icon(markup: string, className?: string): SVGElement {
  const template = document.createElement("template");
  template.innerHTML = markup.trim();
  const node = template.content.firstElementChild as SVGElement;
  if (className) node.setAttribute("class", className);
  return node;
}

function applyProps(element: HTMLElement, props?: Props | null): void {
  if (!props) return;

  for (const [key, value] of Object.entries(props)) {
    if (value === undefined || value === false || value === null) continue;

    if (key === "class") {
      element.className = String(value);
    } else if (key === "html") {
      element.innerHTML = String(value);
    } else if (key === "style" && typeof value === "object") {
      Object.assign(element.style, value);
    } else if (key === "dataset" && typeof value === "object") {
      for (const [name, item] of Object.entries(value as Record<string, unknown>)) {
        if (item !== undefined) element.dataset[name] = String(item);
      }
    } else if (key.startsWith("on") && typeof value === "function") {
      element.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (value === true) {
      element.setAttribute(key, "");
    } else {
      element.setAttribute(key, String(value));
    }
  }
}

function append(parent: Node, children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    parent.appendChild(
      child instanceof Node ? child : document.createTextNode(String(child)),
    );
  }
}

/** Replaces an element's contents in one pass. */
export function fill(parent: Element, ...children: Child[]): void {
  parent.replaceChildren();
  append(parent, children);
}

export function qs<T extends Element = HTMLElement>(selector: string, root: ParentNode = document): T {
  const found = root.querySelector<T>(selector);
  if (!found) throw new Error(`Oracle: no element matches ${selector}`);
  return found;
}

/**
 * Runs `fn` at most once per animation frame.
 *
 * Pointer-move handlers that write CSS variables would otherwise fire far more often than
 * the screen can show, which is pure wasted layout work.
 */
export function onFrame<T extends unknown[]>(fn: (...args: T) => void): (...args: T) => void {
  let queued = false;
  let latest: T;

  return (...args: T) => {
    latest = args;
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      fn(...latest);
    });
  };
}

/** Trailing-edge debounce, for search fields and window resize. */
export function debounce<T extends unknown[]>(
  fn: (...args: T) => void,
  ms: number,
): (...args: T) => void {
  let timer: number | undefined;

  return (...args: T) => {
    if (timer !== undefined) clearTimeout(timer);
    timer = window.setTimeout(() => fn(...args), ms);
  };
}

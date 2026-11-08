// Scrollbars stay hidden until a box is scrolled or the pointer nears its edge.
const edge = 16;
const linger = 900;

export function quietScrollbars() {
  const timers = new WeakMap<Element, number>();
  const show = (el: Element) => {
    el.setAttribute('data-scrolling', '');
    window.clearTimeout(timers.get(el));
    timers.set(
      el,
      window.setTimeout(() => el.removeAttribute('data-scrolling'), linger),
    );
  };
  document.addEventListener(
    'scroll',
    (event) => {
      const target = event.target;
      show(target instanceof Element ? target : document.documentElement);
    },
    { capture: true, passive: true },
  );
  // Near the window's right edge the page scrollbar shows, so it can be grabbed.
  const root = document.documentElement;
  document.addEventListener(
    'pointermove',
    (event) => {
      const near = root.scrollHeight > root.clientHeight && event.clientX > root.clientWidth - edge;
      root.toggleAttribute('data-scroll-near', near);
    },
    { passive: true },
  );
}

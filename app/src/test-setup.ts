// jsdom has no matchMedia; svelte/motion's prefersReducedMotion needs it.
if (!window.matchMedia) {
  window.matchMedia = (query: string): MediaQueryList =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

// jsdom has no canvas, so uPlot cannot draw: every test file gets the recording stub.
vi.mock('uplot', async () => ({ default: (await import('./test/uplot-stub')).FakeUplot }));

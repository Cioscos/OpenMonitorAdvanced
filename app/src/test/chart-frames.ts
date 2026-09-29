import * as clock from '../lib/chartFrameClock';

export interface FrameSubscription {
  fps: number;
  stopped: boolean;
}

/**
 * Spies on `subscribeChartFrame`, keeping the real behaviour, and records each subscription
 * with the rate it asked for and whether it was released.
 */
export function spyChartFrames(): FrameSubscription[] {
  const subscriptions: FrameSubscription[] = [];
  const real = clock.subscribeChartFrame;
  vi.spyOn(clock, 'subscribeChartFrame').mockImplementation((callback, fps = 60) => {
    const record: FrameSubscription = { fps, stopped: false };
    subscriptions.push(record);
    const stop = real(callback, fps);
    return () => {
      record.stopped = true;
      stop();
    };
  });
  return subscriptions;
}

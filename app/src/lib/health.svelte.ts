import type { Backend, Unsubscribe } from './backend/backend';
import { DASH, formatDuration, formatTemperature, formatValue } from './format';
import type { Translate } from './i18n/index.svelte';
import type { Alert, HealthClock, HealthReport, Schema, TemperatureUnit, ThroughputUnit, Unit } from './types';

/**
 * The rules engine's verdict as the core last reported it. Reports are replaced as a whole and
 * only by a newer `revision`; the clock belongs to the report of the same revision.
 */
class HealthStore {
  report = $state.raw<HealthReport | null>(null);
  clock = $state.raw<HealthClock | null>(null);
  /** Bumped by every `connect`, so a superseded connection's late events are dropped. */
  #generation = 0;

  /**
   * Time in the current level, from the core's monotonic clock. Null until a clock of the
   * report's revision arrived (a clock for a newer revision waits for its report).
   */
  get elapsedMs(): number | null {
    const { report, clock } = this;
    return report !== null && clock !== null && clock.revision === report.revision ? clock.levelElapsedMs : null;
  }

  /**
   * Subscribes to `oma:health` and `oma:health-clock` before reading, so a change made in
   * between is not lost, and keeps only reports newer than the current one. Failure removes the
   * listeners it added.
   */
  async connect(backend: Backend): Promise<Unsubscribe> {
    const generation = ++this.#generation;
    this.report = null;
    this.clock = null;
    const offs: Unsubscribe[] = [];
    const stop = () => {
      offs.splice(0).forEach((off) => off());
      if (this.#generation === generation) {
        this.#generation++;
        this.report = null;
        this.clock = null;
      }
    };
    const current = () => this.#generation === generation;
    try {
      offs.push(
        await backend.onHealth((next) => {
          if (current()) this.#acceptReport(next);
        }),
      );
      offs.push(
        await backend.onHealthClock((next) => {
          if (current()) this.#acceptClock(next);
        }),
      );
      const report = await backend.getHealth();
      if (current()) this.#acceptReport(report);
      const clock = await backend.getHealthClock();
      if (current()) this.#acceptClock(clock);
    } catch (error) {
      stop();
      throw error;
    }
    return stop;
  }

  #acceptReport(next: HealthReport): void {
    if (this.report === null || next.revision > this.report.revision) this.report = next;
  }

  #acceptClock(next: HealthClock): void {
    if (this.clock === null || next.revision >= this.clock.revision) this.clock = next;
  }
}

/** The app-wide instance. */
export const health = new HealthStore();

function formatAlertValue(value: number | null, unit: Unit, locale: string, t: Translate, temperature: TemperatureUnit, throughput: ThroughputUnit): string {
  if (unit === 'celsius') return formatTemperature(value, locale, temperature);
  return formatValue(value, unit, locale, t, { rate: throughput });
}

/**
 * Throughput follows the setting on network devices and is shown in bytes elsewhere, like the
 * device pages of the Advanced view and the tray. A device gone from the schema (a retained
 * alert) is recognized by its id.
 */
function rateFor(schema: Schema | null, deviceId: string, throughput: ThroughputUnit): ThroughputUnit {
  const device = schema?.devices.find((d) => d.id === deviceId);
  const network = device === undefined ? deviceId.startsWith('network/') : device.kind === 'network';
  return network ? throughput : 'bytes';
}

function alertMessage(alert: Alert, schema: Schema | null, t: Translate, locale: string, temperature: TemperatureUnit, throughput: ThroughputUnit): string {
  const rate = rateFor(schema, alert.deviceId, throughput);
  const format = (value: number | null) => formatAlertValue(value, alert.unit, locale, t, temperature, rate);
  const device = alert.params.device ?? schema?.devices.find((d) => d.id === alert.deviceId)?.name ?? alert.deviceId;
  const sensor = t(`sensor.${alert.sensorLabel.key}`, alert.sensorLabel.arg === undefined ? {} : { arg: alert.sensorLabel.arg });
  return t(alert.messageKey, {
    ...alert.params,
    device,
    // The value only counts while the sensor still reports; a retained alert says so.
    value: alert.valid && alert.value !== null ? format(alert.value) : t('health.unavailableValue'),
    threshold: alert.threshold === null ? DASH : format(alert.threshold),
    sensor,
    // The volume's own name ("C:"), else the whole label.
    volume: alert.sensorLabel.arg ?? sensor,
  });
}

/**
 * What the Simple view's banner says. One alert: its message; several: "N problems" plus one
 * message per alert in the report's order; none: the coverage decides ("all clear" or
 * "incomplete data"), and a neutral report keeps the monitoring message. Unit and language
 * are arguments so a settings change re-renders without a new report.
 */
export function bannerText(
  report: HealthReport,
  schema: Schema | null,
  t: Translate,
  locale: string,
  temperature: TemperatureUnit,
  throughput: ThroughputUnit,
): { title: string; items: string[] } {
  const messages = report.alerts.map((alert) => alertMessage(alert, schema, t, locale, temperature, throughput));
  if (messages.length === 1) return { title: messages[0], items: [] };
  if (messages.length > 1) return { title: t('health.problems', { count: messages.length }), items: messages };
  if (report.level === 'neutral') return { title: t('health.monitoring'), items: [] };
  return { title: t(report.coverage === 'complete' ? 'health.allClear' : 'health.partial'), items: [] };
}

/** How long the banner's level has lasted: "for 12 min", and "for less than a minute" before the first minute. */
export function sinceText(elapsedMs: number, t: Translate): string {
  if (elapsedMs < 60_000) return t('health.sinceUnderMinute');
  return t('health.since', { duration: formatDuration(elapsedMs, t) });
}

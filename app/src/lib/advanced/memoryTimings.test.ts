import { MOCK_MEMORY_PROPERTIES as THIS_MACHINE } from '../backend/mock';
import { jedecCycles, jedecRate, MAX_CHANNELS, MAX_MODULES, parseMemoryTimings } from './memoryTimings';

test('parses_this_machine', () => {
  const info = parseMemoryTimings(THIS_MACHINE)!;
  const ch = info.timings!.channels[0];
  expect(info.timings!.generation).toBe('ddr5');
  expect([ch.cl, ch.trcdRd, ch.trp, ch.tras].join('-')).toBe('36-48-48-104');
  expect(ch.dataRate).toBe(6000);
  expect(ch.gdm).toBe(true);
  expect(ch.refreshMode).toBe('mixed');
  expect(info.modules.map((m) => m.slot)).toEqual(['DIMM #1', 'DIMM #3']);
  expect(info.modules[0].trfcSbNs).toBe(130);
});

test('rejects_non_numeric_fields', () => {
  const info = parseMemoryTimings({
    memoryModules: JSON.stringify([{ slot: 'A', capacityBytes: '16' }, { slot: 'B', tckMinNs: 0.4 }, { capacityBytes: 1 }, 5, null]),
    memoryTimings: JSON.stringify({ generation: 'ddr5', mclkMhz: 3000, cl: 36, trp: 'x', tras: -1, gdm: 'yes', refreshMode: 'weird' }),
  })!;
  expect(info.modules.map((m) => m.slot)).toEqual(['B']);
  const ch = info.timings!.channels[0];
  expect(ch.trp).toBeUndefined();
  expect(ch.tras).toBeUndefined();
  expect(ch.gdm).toBeUndefined();
  expect(ch.refreshMode).toBeUndefined();
  expect(parseMemoryTimings({ memoryTimings: JSON.stringify({ mclkMhz: 'fast', cl: 36 }) })).toBeNull();
});

test('caps_modules_and_channels', () => {
  const mods = Array.from({ length: 10000 }, (_, i) => ({ slot: `M${i}` }));
  const chans = Array.from({ length: 40 }, (_, i) => ({ channel: String.fromCharCode(65 + (i % 26)), mclkMhz: 3000, cl: 36 }));
  const info = parseMemoryTimings({ memoryModules: JSON.stringify(mods), memoryTimings: JSON.stringify({ generation: 'ddr5', channels: chans }) })!;
  expect(info.modules).toHaveLength(MAX_MODULES);
  expect(info.timings!.channels).toHaveLength(MAX_CHANNELS);
});

test('ignores_unknown_fields', () => {
  const info = parseMemoryTimings({
    memoryModules: JSON.stringify([{ slot: 'A', evil: '<b>x</b>', nested: { a: 1 } }]),
    memoryTimings: JSON.stringify({ generation: 'ddr4', mclkMhz: 1600, cl: 16, extra: [1, 2] }),
  })!;
  expect(info.modules).toEqual([{ slot: 'A' }]);
  expect(info.timings!.channels[0]).toEqual({ mclkMhz: 1600, cl: 16 });
});

test('jedec_cycles_round_up', () => {
  expect(jedecCycles(16, 0.416)).toBe(39);
  expect(jedecCycles(16, 0.4)).toBe(40);
  expect(jedecRate(0.416)).toBe(4808);
});

test('missing_properties_give_null', () => {
  expect(parseMemoryTimings(undefined)).toBeNull();
  expect(parseMemoryTimings({})).toBeNull();
  expect(parseMemoryTimings({ memoryModules: 'not json', memoryTimings: '{' })).toBeNull();
  expect(parseMemoryTimings({ memoryModules: '{}', memoryTimings: '[]' })).toBeNull();
});

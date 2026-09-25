import {
  SECTION_KEY,
  WINDOW_KEY,
  loadSection,
  loadSeries,
  loadWindow,
  saveSection,
  saveSeries,
  saveWindow,
  seriesKey,
} from './persist';

beforeEach(() => localStorage.clear());
afterEach(() => vi.restoreAllMocks());

test('keys follow the oma.advanced.* scheme', () => {
  expect(SECTION_KEY).toBe('oma.advanced.section');
  expect(WINDOW_KEY).toBe('oma.advanced.window');
  expect(seriesKey('gpu/pci-0000:01:00.0')).toBe('oma.advanced.series.gpu/pci-0000:01:00.0');
});

test('section round-trips; missing or empty reads as null', () => {
  expect(loadSection()).toBeNull();
  saveSection('gpu/pci-0000:01:00.0');
  expect(localStorage.getItem(SECTION_KEY)).toBe('gpu/pci-0000:01:00.0');
  expect(loadSection()).toBe('gpu/pci-0000:01:00.0');
  localStorage.setItem(SECTION_KEY, '');
  expect(loadSection()).toBeNull();
});

test('only the four chart windows are accepted', () => {
  expect(loadWindow()).toBeNull();
  saveWindow(1800);
  expect(loadWindow()).toBe(1800);
  for (const bad of ['120', 'abc', '']) {
    localStorage.setItem(WINDOW_KEY, bad);
    expect(loadWindow()).toBeNull();
  }
});

test('series are stored as JSON per section and validated on load', () => {
  expect(loadSeries('cpu/0')).toBeNull();
  saveSeries('cpu/0', ['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(localStorage.getItem('oma.advanced.series.cpu/0')).toBe('["cpu/0/load/total","cpu/0/clock/effective"]');
  expect(loadSeries('cpu/0')).toEqual(['cpu/0/load/total', 'cpu/0/clock/effective']);
  expect(loadSeries('memory/0')).toBeNull();
  for (const bad of ['{', '"x"', '[1,2]', '{"a":1}']) {
    localStorage.setItem(seriesKey('cpu/0'), bad);
    expect(loadSeries('cpu/0')).toBeNull();
  }
});

test('a throwing storage never breaks the view', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('quota');
  });
  expect(() => saveSection('cpu/0')).not.toThrow();
  expect(() => saveWindow(60)).not.toThrow();
  expect(() => saveSeries('cpu/0', [])).not.toThrow();
  expect(loadSection()).toBeNull();
  expect(loadWindow()).toBeNull();
  expect(loadSeries('cpu/0')).toBeNull();
});

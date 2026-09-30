<script lang="ts">
  import type { Action } from 'svelte/action';
  import { sensorLabel } from '../../lib/advanced/labels';
  import { sidebarEntries } from '../../lib/advanced/nav';
  import { categoryLabel, groupSensors } from '../../lib/advanced/pages';
  import { t } from '../../lib/i18n/index.svelte';
  import type { Schema, Sensor } from '../../lib/types';

  // The sensors of the schema as device > category > sensor, each device and category with a
  // three-state box. `selected` may hold ids that are not in the schema now: they stay in the
  // list (the log records them when they appear) and are only counted here.
  let {
    schema,
    selected,
    onChange,
  }: {
    schema: Schema;
    selected: ReadonlySet<string>;
    /** The whole new list: the ids already chosen keep their order, new ones follow. */
    onChange: (next: string[]) => unknown;
  } = $props();

  let query = $state('');
  /** Devices start open; categories start closed (a CPU has dozens of sensors). */
  let closedDevices = $state<Record<string, boolean>>({});
  let openCategories = $state<Record<string, boolean>>({});

  const needle = $derived(query.trim().toLocaleLowerCase());
  const searching = $derived(needle !== '');

  interface CategoryNode {
    key: string;
    name: string;
    sensors: Sensor[];
  }
  interface DeviceNode {
    id: string;
    label: string;
    arg?: string;
    sensors: Sensor[];
    categories: CategoryNode[];
  }

  /** Translated name of every sensor, by id. */
  const names = $derived(new Map(schema.sensors.map((s) => [s.id, sensorLabel(s, t)])));

  const devices = $derived.by<DeviceNode[]>(() => {
    const nodes: DeviceNode[] = [];
    for (const entry of sidebarEntries(schema)) {
      const own = new Set(entry.deviceIds);
      const sensors = schema.sensors.filter(
        (s) => own.has(s.deviceId) && (!searching || (names.get(s.id) ?? '').toLocaleLowerCase().includes(needle)),
      );
      if (sensors.length === 0) continue;
      nodes.push({
        id: entry.id,
        label: t(entry.labelKey),
        arg: entry.labelArg,
        sensors,
        categories: groupSensors(sensors).map((g) => ({ key: `${entry.id}|${g.category}`, name: categoryLabel(g.category, t), sensors: g.sensors })),
      });
    }
    return nodes;
  });

  const present = $derived(new Set(schema.sensors.map((s) => s.id)));
  const missing = $derived([...selected].filter((id) => !present.has(id)));
  const chosen = $derived(schema.sensors.filter((s) => selected.has(s.id)).length);

  type Tri = 'all' | 'some' | 'none';
  const countOn = (sensors: Sensor[]) => sensors.filter((s) => selected.has(s.id)).length;
  function stateOf(sensors: Sensor[]): Tri {
    const on = countOn(sensors);
    return on === 0 ? 'none' : on === sensors.length ? 'all' : 'some';
  }

  /** Clears the group when it is fully chosen, otherwise chooses all of it. */
  function toggle(sensors: Sensor[]) {
    const scope = new Set(sensors.map((s) => s.id));
    if (stateOf(sensors) === 'all') {
      onChange([...selected].filter((id) => !scope.has(id)));
    } else {
      onChange([...selected, ...sensors.filter((s) => !selected.has(s.id)).map((s) => s.id)]);
    }
  }

  /** Sets `indeterminate`, which has no attribute. */
  const tri: Action<HTMLInputElement, Tri> = (node, state) => {
    node.indeterminate = state === 'some';
    return {
      update(next) {
        node.indeterminate = next === 'some';
      },
    };
  };

  const isOpenDevice = (id: string) => searching || !closedDevices[id];
  const isOpenCategory = (key: string) => searching || !!openCategories[key];
</script>

{#snippet chevron(open: boolean, name: string, onclick: () => void)}
  <button
    type="button"
    class="chevron"
    class:open
    aria-label={t('settings.log.sensors.expand', { name })}
    aria-expanded={open}
    disabled={searching}
    {onclick}
  >
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" focusable="false">
      <path d="m6 3 5 5-5 5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
  </button>
{/snippet}

<div class="wrap">
  <div class="bar">
    <input
      type="search"
      class="search"
      bind:value={query}
      placeholder={t('settings.log.sensors.search')}
      aria-label={t('settings.log.sensors.search')}
      autocomplete="off"
      spellcheck="false"
    />
    <p class="count">{t('settings.log.sensors.count', { selected: chosen, total: schema.sensors.length })}</p>
  </div>
  {#if devices.length === 0}
    <p class="empty">{t('settings.log.sensors.none')}</p>
  {:else}
    <ul class="tree" role="tree" aria-label={t('settings.log.sensors.tree')}>
      {#each devices as device (device.id)}
        {@const state = stateOf(device.sensors)}
        <li role="treeitem" aria-selected={state === 'all'} aria-expanded={isOpenDevice(device.id)}>
          <div class="row device">
            {@render chevron(isOpenDevice(device.id), device.arg ?? device.label, () => (closedDevices[device.id] = isOpenDevice(device.id)))}
            <label class="pick">
              <input type="checkbox" checked={state === 'all'} use:tri={state} onchange={() => toggle(device.sensors)} />
              <span class="name">{device.label}{#if device.arg}{' '}<span class="arg">{device.arg}</span>{/if}</span>
            </label>
            <span class="of">{countOn(device.sensors)}/{device.sensors.length}</span>
          </div>
          {#if isOpenDevice(device.id)}
            <ul role="group" class="branch">
              {#each device.categories as category (category.key)}
                {@const cstate = stateOf(category.sensors)}
                <li role="treeitem" aria-selected={cstate === 'all'} aria-expanded={isOpenCategory(category.key)}>
                  <div class="row">
                    {@render chevron(isOpenCategory(category.key), category.name, () => (openCategories[category.key] = !isOpenCategory(category.key)))}
                    <label class="pick">
                      <input type="checkbox" checked={cstate === 'all'} use:tri={cstate} onchange={() => toggle(category.sensors)} />
                      <span class="name">{category.name}</span>
                    </label>
                    <span class="of">{countOn(category.sensors)}/{category.sensors.length}</span>
                  </div>
                  {#if isOpenCategory(category.key)}
                    <ul role="group" class="branch leaves">
                      {#each category.sensors as sensor (sensor.id)}
                        <li role="treeitem" aria-selected={selected.has(sensor.id)}>
                          <label class="pick leaf">
                            <input type="checkbox" checked={selected.has(sensor.id)} onchange={() => toggle([sensor])} />
                            <span class="name">{names.get(sensor.id)}</span>
                          </label>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </li>
              {/each}
            </ul>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if missing.length > 0}
    <p class="missing">{t('settings.log.sensors.missing', { count: missing.length })}</p>
  {/if}
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 16px;
  }
  .search {
    flex: 1 1 200px;
    max-width: 320px;
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .search:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .count,
  .empty,
  .missing {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-muted);
  }
  .missing {
    padding: 6px 10px;
    border-left: 2px solid var(--warn);
    background: color-mix(in srgb, var(--warn) 6%, transparent);
    border-radius: 0 8px 8px 0;
    color: var(--text);
  }
  .tree,
  .branch {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .tree {
    max-height: 420px;
    padding: 4px 8px;
    overflow: auto;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .branch {
    margin-left: 11px;
    padding-left: 6px;
    border-left: 1px solid var(--border);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    min-height: 28px;
  }
  .device .name {
    font-weight: 600;
  }
  .chevron {
    display: inline-grid;
    flex: none;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    color: var(--text-muted);
    cursor: pointer;
    background: none;
    border: 0;
    border-radius: 6px;
  }
  .chevron:disabled {
    cursor: default;
    opacity: 0.4;
  }
  .chevron svg {
    transition: transform 0.12s;
  }
  .chevron.open svg {
    transform: rotate(90deg);
  }
  .chevron:focus-visible,
  .pick input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .pick {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 2px 4px;
    cursor: pointer;
  }
  .leaf {
    min-height: 26px;
    margin-left: 26px;
    font-size: 13px;
  }
  .pick input {
    flex: none;
    margin: 0;
    accent-color: var(--accent);
  }
  .arg {
    font-weight: 400;
    color: var(--text-muted);
  }
  .of {
    flex: none;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
  }
</style>

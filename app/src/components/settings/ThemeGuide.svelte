<script lang="ts">
  import { t } from '../../lib/i18n/index.svelte';
  import variables from '../../themes/variables.json';
  import Term from '../common/Term.svelte';

  // «Create your own theme» (spec M10 §4.3): the parser's real rules, the variable table (from
  // the same `variables.json` the parser uses) and a minimal example. Collapsed by default.
</script>

<details class="guide">
  <summary>{t('themes.guide.title')}</summary>
  <div class="body">
    <p>{t('themes.guide.intro1')}<Term term="cssVariable">{t('glossary.cssVariable.name')}</Term>{t('themes.guide.intro2')}</p>
    <h4>{t('themes.guide.exampleTitle')}</h4>
    <pre class="example"><code>{t('themes.guide.example')}</code></pre>
    <h4>{t('themes.guide.rulesTitle')}</h4>
    <ul>
      <li>
        {t('themes.guide.ruleHeader')}
        <pre class="example"><code>{t('themes.guide.headerExample')}</code></pre>
      </li>
      <li>{t('themes.guide.ruleSize')}</li>
      <li>{t('themes.guide.ruleColors')}<Term term="alpha">{t('glossary.alpha.name')}</Term>{t('themes.guide.ruleColorsAlpha')}</li>
      <li>{t('themes.guide.ruleRadius')}</li>
      <li>{t('themes.guide.ruleFont')}</li>
      <li>{t('themes.guide.ruleDropped')}</li>
    </ul>
    <h4>{t('themes.guide.securityTitle')}</h4>
    <p>{t('themes.guide.security')}</p>
    <h4><Term term="contrast">{t('themes.guide.contrastTitle')}</Term> · <Term term="wcag">WCAG</Term></h4>
    <p>{t('themes.guide.contrastText')}</p>
    <div class="scroll">
      <table>
        <thead>
          <tr>
            <th>{t('themes.guide.colName')}</th>
            <th>{t('themes.guide.colType')}</th>
            <th>{t('themes.guide.colGroup')}</th>
            <th>{t('themes.guide.colMeaning')}</th>
          </tr>
        </thead>
        <tbody>
          {#each variables as v (v.name)}
            <tr>
              <td><code>{v.name}</code></td>
              <td>{t(`themes.kind.${v.kind}`)}</td>
              <td>{t(`themes.group.${v.group}`)}</td>
              <td>{t(`themes.var.${v.name.slice(2)}`)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</details>

<style>
  .guide {
    padding: 10px 14px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius, 8px);
    font-size: 13px;
  }
  summary {
    font-weight: 600;
    cursor: pointer;
  }
  summary:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
  }
  p,
  ul,
  h4 {
    margin: 0;
  }
  h4 {
    margin-top: 6px;
    font-size: 13px;
  }
  ul {
    padding-left: 20px;
  }
  .example {
    margin: 0;
    padding: 8px 12px;
    overflow-x: auto;
    background: var(--surface-2);
    border-radius: 8px;
  }
  code {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
  }
  .scroll {
    overflow-x: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 4px 8px;
    text-align: left;
    vertical-align: top;
    border-bottom: 1px solid var(--border);
  }
  th {
    color: var(--text-muted);
    font-weight: 600;
  }
</style>

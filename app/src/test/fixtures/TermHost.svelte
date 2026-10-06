<script lang="ts">
  import Term from '../../components/common/Term.svelte';

  // Terms as the whole content of blocks, with siblings after them that must survive the blocks' teardown.
  let { show = true, items = [] as string[] }: { show?: boolean; items?: string[] } = $props();
</script>

<p id="if-host">{#if show}<Term term="fft" />{/if}<span id="after-if">after</span> tail</p>
<p id="each-host">{#each items as item (item)}<Term term={item} />{/each}<span id="after-each">after</span></p>

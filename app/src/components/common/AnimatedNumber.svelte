<script lang="ts">
  import { cubicOut } from 'svelte/easing';
  import { Tween, prefersReducedMotion } from 'svelte/motion';

  let { value, format }: { value: number | null; format: (v: number | null) => string } = $props();

  const tween = new Tween(0, {
    duration: () => (prefersReducedMotion.current ? 0 : 300),
    easing: cubicOut,
  });
  let initialized = false;

  $effect(() => {
    if (value === null) return;
    if (!initialized) {
      initialized = true;
      tween.set(value, { duration: 0 });
    } else {
      tween.target = value;
    }
  });
</script>

<span>{value === null ? format(null) : format(tween.current)}</span>

<!--
  "Default" pill for a chart metric toggle — wraps the shared DefaultPill with
  the chartPrefs store. Each call site passes its own `scope`, so every chart's
  default metric is independent (overview / trends_area / trends_rankings).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import DefaultPill from "$lib/dashboard/DefaultPill.svelte";
  import { chartPrefs, type ChartMetric } from "$lib/chart-prefs.svelte";

  type Props = { scope: string; current: ChartMetric };
  let { scope, current }: Props = $props();

  const isDefault = $derived(chartPrefs.defaultMetric(scope) === current);

  onMount(() => {
    chartPrefs.load(scope);
  });
</script>

<DefaultPill
  {isDefault}
  onSet={() => chartPrefs.setDefault(scope, current)}
  isTitle="This metric is your default."
  setTitle="Pin this metric as your default"
/>

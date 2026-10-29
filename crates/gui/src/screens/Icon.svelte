<script lang="ts">
  import type { IconName } from "../lib/icons";

  // Icons go where the mouse clicks and never carry a key badge; a key is shown as a badge and a word in the bar.
  let { name }: { name: IconName } = $props();
</script>

<svg class="i" aria-hidden="true"><use href="#{name}" /></svg>

<script lang="ts">
  import type { Prompt } from "../lib/protocol";
  import { digitChoice } from "../lib/reducer";
  import { reply } from "../lib/store.svelte";

  // The worker's question, over the screen that asked it. Text is typed into real inputs, so the caret,
  // selection and paste are the platform's; the keys the terminal adds are handled here.
  interface Props {
    prompt: Prompt;
  }
  let { prompt }: Props = $props();

  const ESC = { Skip: "skip", Quit: "quit", Keep: "dismiss" };
  const kind = $derived(Object.keys(prompt)[0] as "Choice" | "Form" | "Input");
  const body = $derived(Object.values(prompt)[0] as { header: string; note: string; escape: "Skip" | "Quit" | "Keep"; options?: string[]; fields?: [string, string][]; value?: string });
  let values = $state<string[]>([]);
  let choice = $state(0);
  let inputs: HTMLInputElement[] = $state([]);

  $effect.pre(() => {
    values = kind === "Form" ? body.fields!.map(([, v]) => v) : kind === "Input" ? [body.value!] : [];
    choice = 0;
  });
  $effect(() => {
    inputs[0]?.focus();
  });

  // Found by label, never position, as the terminal's `swappable` does.
  const swap = $derived.by(() => {
    const at = (want: string) => body.fields?.findIndex(([label]) => label === want) ?? -1;
    const pair = [at("artist"), at("title")];
    return pair.every((i) => i >= 0) ? pair : null;
  });

  function key(e: KeyboardEvent) {
    if (e.ctrlKey && (e.key === "t" || e.key === "z" || e.key === "x")) return;
    const options = body.options?.length ?? 0;
    if (e.key === "Escape") return void reply("Cancel");
    if (options) {
      if (e.key === "Enter") return void reply({ Choice: choice });
      if (["ArrowDown", "j", "Tab"].includes(e.key)) choice = (choice + 1) % options;
      else if (["ArrowUp", "k"].includes(e.key)) choice = (choice + options - 1) % options;
      else if (digitChoice(e.key, options) !== null) return void reply({ Choice: digitChoice(e.key, options)! });
      else return;
      e.preventDefault();
      return;
    }
    const at = inputs.indexOf(document.activeElement as HTMLInputElement);
    if (e.key === "Enter") {
      e.preventDefault();
      return void reply(kind === "Form" ? { Fields: [...values] } : { Text: values[0] ?? "" });
    }
    if (e.key === "Tab" || ((e.key === "ArrowDown" || e.key === "ArrowUp") && kind === "Form")) {
      e.preventDefault();
      const step = e.shiftKey || e.key === "ArrowUp" ? -1 : 1;
      inputs[(at + step + inputs.length) % inputs.length]?.focus();
    } else if (e.ctrlKey && e.key === "s" && swap) {
      e.preventDefault();
      [values[swap[0]], values[swap[1]]] = [values[swap[1]], values[swap[0]]];
    } else if (e.ctrlKey && e.key === "u" && at >= 0) {
      // The focused box only: the other holds an answer being kept.
      e.preventDefault();
      values[at] = "";
    }
  }
</script>

<svelte:window onkeydown={key} />

<div class="scrim">
  <div class="dialog">
    <span class="t">{body.header}</span>
    {#if body.note}<p class="note">{body.note}</p>{/if}
    {#if kind === "Choice"}
      <ul class="menu">
        {#each body.options! as option, i (i)}
          <li class:cur={i === choice} onclick={() => reply({ Choice: i })} onkeydown={() => {}} role="option" aria-selected={i === choice}>
            <span class="num">{i + 1}</span><span>{option}</span>
          </li>
        {/each}
      </ul>
    {:else if kind === "Form"}
      <div class="form">
        {#each body.fields! as [name], i (name)}
          <label for="f{i}">{name}</label>
          <div class="box" class:focus={inputs[i] === document.activeElement}>
            <input id="f{i}" bind:this={inputs[i]} bind:value={values[i]} spellcheck="false" />
          </div>
        {/each}
      </div>
    {:else}
      <div class="form" style="grid-template-columns:1fr">
        <div class="box focus"><input bind:this={inputs[0]} bind:value={values[0]} spellcheck="false" /></div>
      </div>
    {/if}
    <p class="foot">
      {#if kind === "Choice"}<span><span class="k">1-{body.options!.length}</span> answer</span><span><span class="k">enter</span> choose</span>
      {:else if kind === "Form"}<span><span class="k">tab</span> field</span>{#if swap}<span><span class="k">^s</span> swap artist and title</span>{/if}<span><span class="k">enter</span> save</span>
      {:else}<span><span class="k">enter</span> answer</span><span><span class="k">^u</span> clear</span>{/if}
      <span><span class="k">esc</span> {ESC[body.escape]}</span>
    </p>
  </div>
</div>

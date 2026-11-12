import { createEffect, createMemo, For, onCleanup, onMount, Show, useContext } from "solid-js";
import { baseName, parentOf, type FolderPicker } from "@octopus/shared/folderPath.ts";
import { Icon } from "./Icon";
import { turnPage } from "./motion";
import { back } from "./nav";
import { Chev, Id, Slot } from "./parts";
import { Sheet, SheetHead, SheetScroller } from "./Sheet";

/** How many of a listing's folders are read ahead of a tap, so going into one lists its folders at once. */
const READ_AHEAD = 24;

/**
 * The folder picker as a sheet: a field the path is typed into above the folders completing it, a tap on one going
 * into it. Choose hands the folder over once the server reads it as one; lowering the sheet any other way cancels.
 */
export function FolderSheet(props: { picker: FolderPicker }) {
  const picker = props.picker;
  let input!: HTMLInputElement;
  let scroller!: HTMLDivElement;

  const choose = () => picker.choose() && back();
  onCleanup(picker.cancel);
  /** Puts the path at `to`, the caret at its end, where the field keeps it in view. */
  const go = (to: string) => {
    picker.setPath(to);
    input.setSelectionRange(to.length, to.length);
  };
  onMount(() => {
    input.focus({ preventScroll: true });
    input.setSelectionRange(picker.path().length, picker.path().length);
  });

  const rows = createMemo(() => new Map(picker.completions().map((row) => [row.into, row])));
  createEffect(() => {
    for (const into of [...rows().keys()].slice(0, READ_AHEAD)) picker.peek(into);
  });
  picker.onTurn((toward) => turnPage(scroller, scroller.cloneNode(true) as HTMLElement, toward, scroller.scrollTop));

  /** A list scrolled by the finger lowers the keyboard, so the list takes the screen it leaves. */
  let touching = false;
  const scrolled = () => touching && input.blur();

  return (
    <Sheet full>
      <SheetHead
        title="Folder"
        end={
          <button class="btn solid" classList={{ inert: picker.folder() === undefined }} onClick={choose}>
            Choose
          </button>
        }
      />
      <label class="field mono typing" classList={{ pending: picker.pending() }}>
        <Slot class="lead">
          <Icon name="folder" />
          <i class="wait" />
        </Slot>
        <span class="entry">
          <span class="root">
            <bdi>{picker.prefix()}</bdi>
          </span>
          <input
            ref={input}
            value={picker.path()}
            style={{ width: `${picker.path().length + 1}ch` }}
            spellcheck={false}
            autocapitalize="off"
            autocorrect="off"
            autocomplete="off"
            enterkeyhint="done"
            aria-label="Folder path"
            onInput={(event) => picker.setPath(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              choose();
            }}
          />
        </span>
        <span
          class="clear"
          classList={{ shown: picker.path() !== "" }}
          role="button"
          aria-label="Clear"
          onClick={() => go("")}
        >
          <i>
            <Icon name="x" />
          </i>
        </span>
      </label>
      <div class="sp2" />
      <div class="folders">
        <div
          ref={(element) => {
            scroller = element;
            useContext(SheetScroller)(element);
          }}
          class="scroll page"
          onTouchStart={() => (touching = true)}
          onTouchEnd={() => (touching = false)}
          onTouchCancel={() => (touching = false)}
          onScroll={scrolled}
        >
          <For each={[...rows().keys()]}>
            {(into) => {
              const name = () => rows().get(into)?.name ?? "";
              const typed = () => picker.stem().length;
              const up = name() === "..";
              return (
                <div class="mi" role="button" onTouchStart={() => picker.peek(into)} onClick={() => go(into)}>
                  <Slot>
                    <Icon name={up ? "back" : "folder"} />
                  </Slot>
                  <span class="name">
                    <b class="hit">{name().slice(0, typed())}</b>
                    {name().slice(typed())}
                  </span>
                  {up ? <Id>{baseName(parentOf(picker.listing().folder))}</Id> : <Chev />}
                </div>
              );
            }}
          </For>
          <Show when={picker.listing().note}>
            {(note) => (
              <Show when={note().refused} fallback={<div class="empty">{note().text}</div>}>
                <div class="note err wrap">
                  <Slot>
                    <Icon name="warn" />
                  </Slot>
                  <span class="name">{note().text}</span>
                </div>
              </Show>
            )}
          </Show>
        </div>
      </div>
    </Sheet>
  );
}

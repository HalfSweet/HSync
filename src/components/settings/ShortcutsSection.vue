<script setup lang="ts">
import { RotateCcw, X } from '@lucide/vue'
import { nextTick, onBeforeUnmount, shallowRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import { useChordLabel } from '@/composables/useShortcuts'
import { SHORTCUT_GROUPS, type ShortcutId } from '@/shortcuts/actions'
import { chordOf, heldLabel, keyCount, MAX_KEYS } from '@/shortcuts/chords'
import { useShortcutsStore } from '@/stores/shortcuts'

/**
 * The settings page's Shortcuts section (DESIGN.md §11): each action with its shortcut in a box,
 * grouped as File, View and Capture. Clicking the box records the keys pressed together next; a
 * chord another action had moves here, and the row says so.
 */
const { t } = useI18n()
const shortcuts = useShortcutsStore()
const label = useChordLabel()
const root = useTemplateRef<HTMLElement>('root')

const actionName = (id: ShortcutId) => t(`settings.shortcuts.actions.${id}`)

/** The action whose chord is being recorded, and the modifiers held so far. */
const recording = shallowRef<{ id: ShortcutId; held: string | null } | null>(null)
/** What the last change did, under the row it was made in. */
const note = shallowRef<{ id: ShortcutId; text: string } | null>(null)
/** A key ended a recording: its release must not press the box again. */
let ended = false

async function focusBox(id: ShortcutId): Promise<void> {
  await nextTick()
  root.value?.querySelector<HTMLElement>(`[data-shortcut="${id}"]`)?.focus()
}

function record(id: ShortcutId): void {
  note.value = null
  recording.value = { id, held: null }
  shortcuts.recording = true
  // WebKit leaves the focus where it was on a click: the box needs it for the keys.
  void focusBox(id)
}

function stop(): void {
  recording.value = null
  shortcuts.recording = false
}
onBeforeUnmount(stop)

function onKeyDown(id: ShortcutId, event: KeyboardEvent): void {
  if (recording.value?.id !== id) return
  // Tab still moves the focus on, so the keyboard never gets stuck here.
  if (event.key === 'Tab' && !event.ctrlKey && !event.metaKey && !event.altKey) {
    stop()
    return
  }
  // The keys are the recording's alone: no shortcut, and no Esc closing the settings.
  event.preventDefault()
  event.stopPropagation()
  const plain = !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey
  if (event.key === 'Escape' && plain) {
    ended = true
    stop()
    return
  }
  const chord = chordOf(event)
  if (chord === null) {
    // Modifiers alone: show them, and wait for the key.
    recording.value = { id, held: heldLabel(event) }
    return
  }
  if (keyCount(chord) > MAX_KEYS) {
    note.value = { id, text: t('settings.shortcuts.tooMany', { count: MAX_KEYS }) }
    return
  }
  const from = shortcuts.assign(id, chord)
  note.value = from && {
    id,
    text: t('settings.shortcuts.taken', { chord: label(chord), action: actionName(from) }),
  }
  ended = true
  stop()
}

function onKeyUp(id: ShortcutId, event: KeyboardEvent): void {
  if (recording.value?.id === id) {
    event.preventDefault()
    recording.value = { id, held: heldLabel(event) }
  } else if (ended) {
    // Space presses a button when it is let go: not the box that just took it.
    event.preventDefault()
    ended = false
  }
}

function clear(id: ShortcutId): void {
  note.value = null
  shortcuts.clear(id)
  void focusBox(id)
}

function reset(id: ShortcutId): void {
  const from = shortcuts.reset(id)
  note.value = from && {
    id,
    text: t('settings.shortcuts.takenBack', { action: actionName(from) }),
  }
  void focusBox(id)
}

function resetAll(): void {
  note.value = null
  shortcuts.resetAll()
}
</script>

<template>
  <div ref="root" class="flex flex-col gap-5">
    <div class="flex items-start justify-between gap-4">
      <p class="text-ink-3">{{ t('settings.shortcuts.intro', { count: MAX_KEYS }) }}</p>
      <button
        class="btn-ghost h-[22px] flex-none"
        :disabled="!shortcuts.anyChanged"
        @click="resetAll"
      >
        <RotateCcw :size="14" />
        {{ t('settings.shortcuts.resetAll') }}
      </button>
    </div>

    <section
      v-for="(ids, group) in SHORTCUT_GROUPS"
      :key="group"
      :aria-labelledby="`shortcuts-${group}`"
      class="flex flex-col gap-1"
    >
      <h3 :id="`shortcuts-${group}`" class="text-caption font-medium text-ink-3">
        {{ t(`settings.shortcuts.groups.${group}`) }}
      </h3>
      <ul class="flex flex-col">
        <li v-for="id in ids" :key="id" class="flex flex-col py-1">
          <div class="flex items-center gap-3">
            <span class="w-48 flex-none text-ink-2">{{ actionName(id) }}</span>
            <span
              class="flex h-[26px] w-44 flex-none items-center rounded-4"
              :class="recording?.id === id ? 'bg-ink text-canvas' : 'bg-control'"
            >
              <button
                :data-shortcut="id"
                class="h-full min-w-0 flex-1 truncate rounded-4 px-2.5 text-left tabular-nums"
                :class="recording?.id !== id && 'hover:bg-control-hover'"
                :aria-label="
                  recording?.id === id
                    ? t('settings.shortcuts.pressHint')
                    : t('settings.shortcuts.change', {
                        action: actionName(id),
                        chord: shortcuts.bindings[id]
                          ? label(shortcuts.bindings[id])
                          : t('settings.shortcuts.none'),
                      })
                "
                @click="record(id)"
                @keydown="onKeyDown(id, $event)"
                @keyup="onKeyUp(id, $event)"
                @blur="recording?.id === id && stop()"
              >
                <template v-if="recording?.id === id">{{
                  recording.held ?? t('settings.shortcuts.press')
                }}</template>
                <kbd v-else-if="shortcuts.bindings[id]" class="font-sans">{{
                  label(shortcuts.bindings[id])
                }}</kbd>
                <span v-else class="text-ink-3">{{ t('settings.shortcuts.none') }}</span>
              </button>
              <button
                v-if="shortcuts.bindings[id] && recording?.id !== id"
                class="mr-0.5 inline-flex size-[22px] flex-none items-center justify-center rounded-4 text-ink-3 hover:bg-control-hover hover:text-ink"
                :aria-label="t('settings.shortcuts.clear', { action: actionName(id) })"
                :title="t('settings.shortcuts.clear', { action: actionName(id) })"
                @click="clear(id)"
              >
                <X :size="12" />
              </button>
            </span>
            <button
              v-if="shortcuts.changed(id)"
              class="btn-icon size-[22px] flex-none"
              :aria-label="t('settings.shortcuts.reset', { action: actionName(id) })"
              :title="t('settings.shortcuts.reset', { action: actionName(id) })"
              @click="reset(id)"
            >
              <RotateCcw :size="14" />
            </button>
          </div>
          <p
            v-if="note?.id === id"
            role="status"
            class="rise-in pt-0.5 pl-[calc(12rem+0.75rem)] text-caption text-ink-3"
          >
            {{ note.text }}
          </p>
        </li>
      </ul>
    </section>
  </div>
</template>

<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import { useBackendText } from '@/composables/useBackendText'
import { toUiError, type UiError } from '@/ipc/errors'
import { useDecodeStore } from '@/stores/decode'

const { t } = useI18n()
const decode = useDecodeStore()
const text = useBackendText()
const error = shallowRef<UiError | null>(null)
const selected = shallowRef(0)
const current = computed(() => decode.importWarnings[selected.value])
const diagnostic = shallowRef(0)
const message = computed(() => current.value?.diagnostics[diagnostic.value])
const removing = shallowRef(false)
watch(
  () => decode.importWarnings.length,
  (length) => {
    selected.value = Math.max(0, Math.min(selected.value, length - 1))
  },
)
watch(current, () => {
  diagnostic.value = 0
})

async function remove(): Promise<void> {
  if (!current.value) return
  removing.value = true
  error.value = null
  try {
    await decode.setEntries(decode.entries.filter((_, index) => index !== current.value?.index))
  } catch (reason) {
    error.value = toUiError(reason)
  } finally {
    removing.value = false
  }
}
</script>

<template>
  <details v-if="current" class="shrink-0 border-b border-line bg-chrome px-3 py-1 text-ink-2">
    <summary class="cursor-pointer text-alert">
      {{ t('decode.import.summary', { count: decode.importWarnings.length }) }}
    </summary>
    <div class="flex flex-wrap items-center gap-2 py-1">
      <span class="min-w-0 truncate">{{ current.label }}</span>
      <span class="tabular-nums">{{ selected + 1 }} / {{ decode.importWarnings.length }}</span>
      <button
        class="rounded-6 bg-control px-2 py-1 disabled:opacity-40"
        :disabled="selected === 0"
        @click="selected--"
      >
        {{ t('decode.import.previous') }}
      </button>
      <button
        class="rounded-6 bg-control px-2 py-1 disabled:opacity-40"
        :disabled="selected + 1 >= decode.importWarnings.length"
        @click="selected++"
      >
        {{ t('decode.import.next') }}
      </button>
      <button
        class="rounded-6 bg-control px-2 py-1 disabled:opacity-40"
        :disabled="removing"
        @click="remove"
      >
        {{ t('decode.import.remove') }}
      </button>
    </div>
    <p v-if="error" role="alert" class="pb-1 text-alert">{{ text.error(error) }}</p>
    <p class="pb-1">{{ t('decode.import.retained') }}</p>
    <div v-if="message" class="flex flex-wrap items-center gap-2 pb-1">
      <span>{{ text.error(message) }}</span>
      <template v-if="current.diagnostics.length > 1">
        <span class="tabular-nums">{{ diagnostic + 1 }} / {{ current.diagnostics.length }}</span>
        <button
          class="rounded-6 bg-control px-2 py-1 disabled:opacity-40"
          :disabled="diagnostic === 0"
          @click="diagnostic--"
        >
          {{ t('decode.import.previousDetail') }}
        </button>
        <button
          class="rounded-6 bg-control px-2 py-1 disabled:opacity-40"
          :disabled="diagnostic + 1 >= current.diagnostics.length"
          @click="diagnostic++"
        >
          {{ t('decode.import.nextDetail') }}
        </button>
      </template>
    </div>
  </details>
</template>

import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import StreamWindow from './StreamWindow.vue'
import { i18n } from './i18n'
import './styles/main.css'
import { inStreamWindow } from './windows/role'

// The data stream's own window loads the same page, told to show the data stream alone.
createApp(inStreamWindow ? StreamWindow : App)
  .use(createPinia())
  .use(i18n)
  .mount('#app')

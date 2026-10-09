import { createPinia } from 'pinia'
import { createApp } from 'vue'

import App from './App.vue'
import { i18n } from './i18n'
import './styles/main.css'

createApp(App).use(createPinia()).use(i18n).mount('#app')

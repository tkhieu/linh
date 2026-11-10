import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'

import '../styles.css'
import { PromptApp } from './PromptApp'

document.documentElement.classList.add('prompt-page')

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <PromptApp />
  </StrictMode>,
)

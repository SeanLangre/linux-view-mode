import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './styles.css'
import ModeGate from './ModeGate.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ModeGate />
  </StrictMode>,
)

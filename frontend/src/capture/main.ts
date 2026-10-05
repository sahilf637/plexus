import { mount } from 'svelte'
import '../app.css'
import CaptureApp from './CaptureApp.svelte'

export default mount(CaptureApp, { target: document.getElementById('app')! })

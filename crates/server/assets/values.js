// Values form for index.html: GET /values fills the inputs, submitting PUTs
// them back. The server fills them into the layout template, so the canvas
// (driven by preview.js polling GET /screen) shows the result on refresh.

'use strict';

const MAX_TO_EAT = 8;

const form = document.getElementById('values');
const todoEl = form.elements.todo;
const mealEls = Array.from(form.querySelectorAll('input[name=meal]'));
const toEatBox = document.getElementById('to-eat');
const toEatEls = [];
for (let i = 0; i < MAX_TO_EAT; i++) {
  const input = document.createElement('input');
  input.type = 'text';
  input.autocomplete = 'off';
  input.setAttribute('aria-label', `To Eat item ${i + 1}`);
  toEatBox.appendChild(input);
  toEatEls.push(input);
}

async function loadValues() {
  const res = await fetch('/values', { cache: 'no-store' });
  if (!res.ok) {
    putResult.textContent = `GET /values failed: ${res.status}`;
    putResult.className = 'err';
    return;
  }
  const values = await res.json();
  todoEl.value = values.todo || '';
  mealEls.forEach((el, i) => { el.value = (values.meals || [])[i] || ''; });
  toEatEls.forEach((el, i) => { el.value = (values.to_eat || [])[i] || ''; });
}

async function putValues(ev) {
  ev.preventDefault();
  const body = JSON.stringify({
    todo: todoEl.value.trim(),
    meals: mealEls.map((el) => el.value.trim()),
    to_eat: toEatEls.map((el) => el.value.trim()).filter((v) => v),
  });
  putResult.textContent = 'sending…';
  putResult.className = '';
  try {
    const res = await fetch('/values', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body,
    });
    if (res.ok) {
      putResult.textContent = `accepted (${res.status})`;
      await loadValues();
      await refresh(true);
    } else {
      putResult.textContent = `${res.status}: ${await res.text()}`;
      putResult.className = 'err';
    }
  } catch (e) {
    putResult.textContent = `PUT failed: ${e}`;
    putResult.className = 'err';
  }
}

form.addEventListener('submit', putValues);
// Select the whole value on focus so typing replaces it. Deferred so the
// click's own mouseup doesn't immediately collapse the selection again.
form.addEventListener('focusin', (ev) => {
  if (ev.target.matches('input[type=text]')) setTimeout(() => ev.target.select(), 0);
});
loadValues();

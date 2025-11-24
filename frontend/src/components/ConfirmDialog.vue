<script setup lang="ts">
interface Props {
  show: boolean
  title?: string
  message: string
  confirmText?: string
  cancelText?: string
}

const props = withDefaults(defineProps<Props>(), {
  title: '确认操作',
  confirmText: '确定',
  cancelText: '取消'
})

const emit = defineEmits<{
  confirm: []
  cancel: []
  'update:show': [value: boolean]
}>()

function handleConfirm() {
  emit('confirm')
  emit('update:show', false)
}

function handleCancel() {
  emit('cancel')
  emit('update:show', false)
}

function handleOverlayClick() {
  handleCancel()
}
</script>

<template>
  <Transition name="modal">
    <div v-if="show" class="modal-overlay" @click="handleOverlayClick">
      <div class="modal-dialog" @click.stop>
        <div class="modal-header">
          <h4>{{ title }}</h4>
          <button class="close-btn" @click="handleCancel">✕</button>
        </div>
        
        <div class="modal-body">
          <p>{{ message }}</p>
        </div>
        
        <div class="modal-footer">
          <button class="btn btn-cancel" @click="handleCancel">
            {{ cancelText }}
          </button>
          <button class="btn btn-confirm" @click="handleConfirm">
            {{ confirmText }}
          </button>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 9999;
}

.modal-dialog {
  background: white;
  border-radius: 8px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
  max-width: 420px;
  width: 90%;
  overflow: hidden;
}

.modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 16px 24px;
  background: #f8f9fa;
  border-bottom: 1px solid #e0e0e0;
}

.modal-header h4 {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
  color: #333;
}

.close-btn {
  background: none;
  border: none;
  font-size: 20px;
  color: #666;
  cursor: pointer;
  padding: 4px;
  line-height: 1;
  border-radius: 4px;
  transition: background 0.2s ease;
}

.close-btn:hover {
  background: #e0e0e0;
}

.modal-body {
  padding: 24px;
}

.modal-body p {
  margin: 0;
  font-size: 14px;
  line-height: 1.6;
  color: #333;
}

.modal-footer {
  display: flex;
  gap: 12px;
  justify-content: flex-end;
  padding: 16px 24px;
  border-top: 1px solid #f0f0f0;
}

.btn {
  padding: 8px 16px;
  border-radius: 4px;
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.2s ease;
  border: 1px solid;
}

.btn-cancel {
  background: white;
  color: #333;
  border-color: #ddd;
}

.btn-cancel:hover {
  border-color: #999;
  background: #f8f9fa;
}

.btn-confirm {
  background: #2196f3;
  color: white;
  border-color: #2196f3;
}

.btn-confirm:hover {
  background: #1976d2;
  border-color: #1976d2;
}

/* Transition */
.modal-enter-active,
.modal-leave-active {
  transition: opacity 0.2s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}

.modal-enter-active .modal-dialog,
.modal-leave-active .modal-dialog {
  transition: transform 0.2s ease;
}

.modal-enter-from .modal-dialog,
.modal-leave-to .modal-dialog {
  transform: scale(0.95);
}
</style>

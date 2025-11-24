import { ElNotification } from 'element-plus'
import 'element-plus/theme-chalk/el-notification.css'

export function useNotify() {
  const showMessage = (
    message: string,
    type: 'success' | 'info' | 'warning' | 'error' = 'info',
    title?: string,
    options: any = {}
  ) => {
    const instance = ElNotification({
      title: title || (type === 'error' ? '错误' : type === 'success' ? '成功' : type === 'warning' ? '警告' : '提示'),
      message,
      type,
      duration: 0, // Disable auto-close by Element Plus
      position: 'top-right',
      showClose: true,
      customClass: 'custom-notification', 
      ...options,
    })

    // Manually close after 2000ms regardless of hover
    setTimeout(() => {
      instance.close()
    }, 2000)
  }

  const success = (msg: string, title?: string) => showMessage(msg, 'success', title)
  const error = (msg: string, title?: string) => showMessage(msg, 'error', title)
  const warning = (msg: string, title?: string) => showMessage(msg, 'warning', title)
  const info = (msg: string, title?: string) => showMessage(msg, 'info', title)

  return {
    showMessage,
    success,
    error,
    warning,
    info,
  }
}

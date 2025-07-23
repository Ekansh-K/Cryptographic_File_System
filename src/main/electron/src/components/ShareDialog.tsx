import React, { useState, useEffect } from 'react'
import {
  Modal,
  Form,
  Input,
  Select,
  DatePicker,
  Switch,
  Button,
  message,
  AutoComplete,
  Tag,
  Space,
  Divider,
  Typography,
  Alert,
  Spin,
} from 'antd'
import {
  ShareAltOutlined,
  UserOutlined,
  LockOutlined,
  CalendarOutlined,
  InfoCircleOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import dayjs from 'dayjs'
import { Container, ShareConfig, SharePermission, SharedContainer, userAPI, sharingAPI } from '../services/api'
import { auditService, AuditEventType } from '../services/auditService'

const { Option } = Select
const { TextArea } = Input
const { Text, Title } = Typography

interface ShareDialogProps {
  visible: boolean
  container: Container | null
  onShare: (shareConfig: ShareConfig) => Promise<SharedContainer | void>
  onCancel: () => void
}

interface UserSearchOption {
  value: string
  label: string
  username: string
}

const ShareDialog: React.FC<ShareDialogProps> = ({
  visible,
  container,
  onShare,
  onCancel,
}) => {
  const [form] = Form.useForm()
  const [loading, setLoading] = useState(false)
  const [searchLoading, setSearchLoading] = useState(false)
  const [userOptions, setUserOptions] = useState<UserSearchOption[]>([])
  const [selectedUser, setSelectedUser] = useState<string>('')
  const [permissions, setPermissions] = useState<SharePermission[]>([SharePermission.READ])
  const [hasExpiration, setHasExpiration] = useState(false)

  // Reset form when dialog opens/closes
  useEffect(() => {
    if (visible) {
      form.resetFields()
      setSelectedUser('')
      setPermissions([SharePermission.READ])
      setHasExpiration(false)
      setUserOptions([])
    }
  }, [visible, form])

  // Search users for autocomplete
  const handleUserSearch = async (searchText: string) => {
    if (!searchText || searchText.length < 2) {
      setUserOptions([])
      return
    }

    setSearchLoading(true)
    try {
      const result = await userAPI.searchUsers(searchText, 10)
      const options = result.users.map(user => ({
        value: user.username,
        label: user.username,
        username: user.username,
      }))
      setUserOptions(options)
    } catch (error) {
      console.error('Failed to search users:', error)
      message.error('Failed to search users')
    } finally {
      setSearchLoading(false)
    }
  }

  // Handle user selection
  const handleUserSelect = (value: string) => {
    setSelectedUser(value)
  }

  // Handle permission changes
  const handlePermissionChange = (value: SharePermission[]) => {
    setPermissions(value)
  }

  // Handle form submission
  const handleSubmit = async () => {
    try {
      const values = await form.validateFields()
      
      if (!selectedUser) {
        message.error('Please select a user to share with')
        return
      }

      setLoading(true)

      const shareConfig: ShareConfig = {
        recipientUsername: selectedUser,
        permissions,
        message: values.message,
        maxAccess: values.maxAccess,
      }

      if (hasExpiration && values.expiresAt) {
        shareConfig.expiresAt = values.expiresAt.toISOString()
      }

      const result = await onShare(shareConfig)
      
      // Log audit event for share creation
      if (container) {
        await auditService.logEvent(
          AuditEventType.SHARE_CREATED,
          result?.id || 'unknown', // Share ID from result
          container.id,
          {
            recipientUsername: selectedUser,
            permissions,
            expiresAt: shareConfig.expiresAt,
            message: shareConfig.message,
            maxAccess: shareConfig.maxAccess
          }
        )
      }
      
      message.success(`Container shared with ${selectedUser} successfully`)
      onCancel()
    } catch (error: any) {
      console.error('Failed to share container:', error)
      message.error(error.message || 'Failed to share container')
    } finally {
      setLoading(false)
    }
  }

  // Permission options with descriptions
  const permissionOptions = [
    {
      value: SharePermission.READ,
      label: 'Read',
      description: 'View and download files',
      color: 'blue',
    },
    {
      value: SharePermission.WRITE,
      label: 'Write',
      description: 'Modify and upload files',
      color: 'orange',
    },
    {
      value: SharePermission.SHARE,
      label: 'Share',
      description: 'Share with other users',
      color: 'purple',
    },
  ]

  return (
    <Modal
      title={
        <div className="flex items-center space-x-2">
          <ShareAltOutlined className="text-primary-500" />
          <span>Share Container</span>
        </div>
      }
      open={visible}
      onCancel={onCancel}
      footer={null}
      width={600}
      className="share-dialog"
      destroyOnClose
    >
      <AnimatePresence>
        {visible && (
          <motion.div
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -20 }}
            transition={{ duration: 0.3 }}
          >
            {container && (
              <div className="mb-6 p-4 bg-gray-50 dark:bg-gray-800 rounded-lg">
                <div className="flex items-center space-x-3">
                  <div className="w-10 h-10 bg-primary-100 dark:bg-primary-900 rounded-lg flex items-center justify-center">
                    <LockOutlined className="text-primary-500" />
                  </div>
                  <div>
                    <Title level={5} className="mb-0">
                      {container.name}
                    </Title>
                    <Text type="secondary" className="text-sm">
                      {(container.size / (1024 * 1024 * 1024)).toFixed(2)} GB • {container.status}
                    </Text>
                  </div>
                </div>
              </div>
            )}

            <Form
              form={form}
              layout="vertical"
              onFinish={handleSubmit}
              className="space-y-4"
            >
              {/* User Selection */}
              <Form.Item
                label={
                  <div className="flex items-center space-x-2">
                    <UserOutlined />
                    <span>Share with User</span>
                  </div>
                }
                name="recipientUsername"
                rules={[
                  { required: true, message: 'Please select a user to share with' },
                ]}
              >
                <AutoComplete
                  placeholder="Search for username..."
                  onSearch={handleUserSearch}
                  onSelect={handleUserSelect}
                  options={userOptions}
                  notFoundContent={searchLoading ? <Spin size="small" /> : 'No users found'}
                  className="w-full"
                  size="large"
                  allowClear
                />
              </Form.Item>

              {/* Permissions */}
              <Form.Item
                label="Permissions"
                name="permissions"
                initialValue={[SharePermission.READ]}
              >
                <Select
                  mode="multiple"
                  placeholder="Select permissions"
                  value={permissions}
                  onChange={handlePermissionChange}
                  size="large"
                  className="w-full"
                >
                  {permissionOptions.map(option => (
                    <Option key={option.value} value={option.value}>
                      <div className="flex items-center justify-between">
                        <div className="flex items-center space-x-2">
                          <Tag color={option.color}>{option.label}</Tag>
                          <span>{option.description}</span>
                        </div>
                      </div>
                    </Option>
                  ))}
                </Select>
              </Form.Item>

              {/* Expiration Toggle */}
              <Form.Item label="Expiration">
                <div className="flex items-center space-x-3">
                  <Switch
                    checked={hasExpiration}
                    onChange={setHasExpiration}
                    checkedChildren="Yes"
                    unCheckedChildren="No"
                  />
                  <Text type="secondary">Set expiration date</Text>
                </div>
              </Form.Item>

              {/* Expiration Date */}
              {hasExpiration && (
                <motion.div
                  initial={{ opacity: 0, height: 0 }}
                  animate={{ opacity: 1, height: 'auto' }}
                  exit={{ opacity: 0, height: 0 }}
                  transition={{ duration: 0.3 }}
                >
                  <Form.Item
                    name="expiresAt"
                    rules={[
                      { required: hasExpiration, message: 'Please select expiration date' },
                    ]}
                  >
                    <DatePicker
                      showTime
                      placeholder="Select expiration date"
                      disabledDate={(current) => current && current < dayjs().endOf('day')}
                      size="large"
                      className="w-full"
                      suffixIcon={<CalendarOutlined />}
                    />
                  </Form.Item>
                </motion.div>
              )}

              {/* Access Limit */}
              <Form.Item
                label="Access Limit (Optional)"
                name="maxAccess"
                tooltip="Maximum number of times the container can be accessed"
              >
                <Input
                  type="number"
                  placeholder="Unlimited"
                  min={1}
                  size="large"
                  suffix="accesses"
                />
              </Form.Item>

              {/* Message */}
              <Form.Item
                label="Message (Optional)"
                name="message"
              >
                <TextArea
                  placeholder="Add a message for the recipient..."
                  rows={3}
                  maxLength={500}
                  showCount
                />
              </Form.Item>

              <Divider />

              {/* Security Notice */}
              <Alert
                message="Security Notice"
                description="The recipient will have access to the container according to the permissions you've selected. You can revoke access at any time from the sharing management page."
                type="info"
                icon={<InfoCircleOutlined />}
                showIcon
                className="mb-4"
              />

              {/* Action Buttons */}
              <div className="flex justify-end space-x-3">
                <Button
                  size="large"
                  onClick={onCancel}
                  disabled={loading}
                >
                  Cancel
                </Button>
                <Button
                  type="primary"
                  size="large"
                  loading={loading}
                  onClick={handleSubmit}
                  icon={<ShareAltOutlined />}
                  className="bg-gradient-to-r from-primary-500 to-primary-600"
                >
                  Share Container
                </Button>
              </div>
            </Form>
          </motion.div>
        )}
      </AnimatePresence>
    </Modal>
  )
}

export default ShareDialog
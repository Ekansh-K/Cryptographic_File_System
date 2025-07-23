import { useState } from 'react'
import {
  Card,
  Row,
  Col,
  Switch,
  Select,
  Slider,
  Button,
  Space,
  Divider,
  Typography,
  Alert,
  Form,
  Input,
  InputNumber,
  message,
  Tabs,
} from 'antd'
import {
  SettingOutlined,
  SecurityScanOutlined,
  BgColorsOutlined,
  BellOutlined,
  FolderOutlined,
  SaveOutlined,
  ReloadOutlined,
} from '@ant-design/icons'
import { motion } from 'framer-motion'
import { useTheme } from '../contexts/ThemeContext'

const { Title, Text } = Typography
const { Option } = Select

interface SettingsData {
  // Security Settings
  autoLockTimeout: number
  maxFailedAttempts: number
  enableIntegrityChecking: boolean
  enableTamperDetection: boolean
  
  // Performance Settings
  encryptionThreads: number
  cacheSize: number
  enableHardwareAcceleration: boolean
  
  // UI Settings
  theme: 'light' | 'dark' | 'system'
  enableAnimations: boolean
  enableNotifications: boolean
  defaultContainerSize: number
  
  // Advanced Settings
  logLevel: string
  enableDebugMode: boolean
  backupLocation: string
}

function Settings() {
  const { theme, setTheme } = useTheme()
  const [form] = Form.useForm<SettingsData>()
  const [loading, setLoading] = useState(false)

  const handleSave = async () => {
    try {
      setLoading(true)
      const values = await form.validateFields()
      
      // Apply theme change immediately
      if (values.theme !== theme) {
        setTheme(values.theme)
      }
      
      // Simulate saving settings
      await new Promise(resolve => setTimeout(resolve, 1000))
      
      message.success('Settings saved successfully!')
    } catch (error) {
      console.error('Failed to save settings:', error)
      message.error('Failed to save settings')
    } finally {
      setLoading(false)
    }
  }

  const handleReset = () => {
    form.resetFields()
    message.info('Settings reset to defaults')
  }

  const containerVariants = {
    hidden: { opacity: 0, y: 20 },
    visible: (i: number) => ({
      opacity: 1,
      y: 0,
      transition: {
        delay: i * 0.1,
        duration: 0.5,
        ease: 'easeOut',
      },
    }),
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.5 }}
      className="space-y-6"
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <Title level={2} className="mb-2">
            Settings
          </Title>
          <Text className="text-gray-600 dark:text-gray-400">
            Configure your EFS preferences and security settings
          </Text>
        </div>
        <Space>
          <Button
            icon={<ReloadOutlined />}
            onClick={handleReset}
            disabled={loading}
          >
            Reset
          </Button>
          <Button
            type="primary"
            icon={<SaveOutlined />}
            onClick={handleSave}
            loading={loading}
          >
            Save Settings
          </Button>
        </Space>
      </div>

      <Card>
        <Form
          form={form}
          layout="vertical"
          initialValues={{
            autoLockTimeout: 30,
            maxFailedAttempts: 3,
            enableIntegrityChecking: true,
            enableTamperDetection: true,
            encryptionThreads: 4,
            cacheSize: 256,
            enableHardwareAcceleration: true,
            theme: theme,
            enableAnimations: true,
            enableNotifications: true,
            defaultContainerSize: 1024,
            logLevel: 'INFO',
            enableDebugMode: false,
            backupLocation: '',
          }}
        >
          <Tabs
            defaultActiveKey="security"
            type="card"
            size="large"
            items={[
              {
                key: 'security',
                label: (
                  <span>
                    <SecurityScanOutlined />
                    Security
                  </span>
                ),
                children: (
                  <motion.div
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                  >
                    <Row gutter={[24, 24]}>
                      <Col span={24}>
                        <Space direction="vertical" className="w-full" size="large">
                          <div>
                            <Form.Item
                              name="autoLockTimeout"
                              label="Auto-lock Timeout (minutes)"
                              help="Automatically lock containers after inactivity"
                            >
                              <Slider
                                min={5}
                                max={120}
                                marks={{
                                  5: '5m',
                                  30: '30m',
                                  60: '1h',
                                  120: '2h',
                                }}
                                tooltip={{
                                  formatter: (value) => `${value} minutes`,
                                }}
                              />
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="maxFailedAttempts"
                              label="Max Failed Login Attempts"
                              help="Lock container after this many failed attempts"
                            >
                              <InputNumber
                                min={1}
                                max={10}
                                style={{ width: '100%' }}
                              />
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="enableIntegrityChecking"
                              label="Enable Integrity Checking"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Automatically verify container integrity on mount
                            </Text>
                          </div>

                          <div>
                            <Form.Item
                              name="enableTamperDetection"
                              label="Enable Tamper Detection"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Monitor for unauthorized access attempts
                            </Text>
                          </div>
                        </Space>
                      </Col>
                    </Row>
                  </motion.div>
                ),
              },
              {
                key: 'performance',
                label: (
                  <span>
                    <SettingOutlined />
                    Performance
                  </span>
                ),
                children: (
                  <motion.div
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                  >
                    <Row gutter={[24, 24]}>
                      <Col span={24}>
                        <Space direction="vertical" className="w-full" size="large">
                          <div>
                            <Form.Item
                              name="encryptionThreads"
                              label="Encryption Threads"
                              help="Number of threads for encryption operations"
                            >
                              <Slider
                                min={1}
                                max={16}
                                marks={{
                                  1: '1',
                                  4: '4',
                                  8: '8',
                                  16: '16',
                                }}
                              />
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="cacheSize"
                              label="Cache Size (MB)"
                              help="Memory cache for frequently accessed files"
                            >
                              <Select style={{ width: '100%' }}>
                                <Option value={128}>128 MB</Option>
                                <Option value={256}>256 MB</Option>
                                <Option value={512}>512 MB</Option>
                                <Option value={1024}>1 GB</Option>
                                <Option value={2048}>2 GB</Option>
                              </Select>
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="enableHardwareAcceleration"
                              label="Hardware Acceleration"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Use hardware acceleration for encryption
                            </Text>
                          </div>
                        </Space>
                      </Col>
                    </Row>
                  </motion.div>
                ),
              },
              {
                key: 'interface',
                label: (
                  <span>
                    <BgColorsOutlined />
                    Interface
                  </span>
                ),
                children: (
                  <motion.div
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                  >
                    <Row gutter={[24, 24]}>
                      <Col span={24}>
                        <Space direction="vertical" className="w-full" size="large">
                          <div>
                            <Form.Item
                              name="theme"
                              label="Theme"
                              help="Choose your preferred theme"
                            >
                              <Select style={{ width: '100%' }}>
                                <Option value="light">Light</Option>
                                <Option value="dark">Dark</Option>
                                <Option value="system">System</Option>
                              </Select>
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="enableAnimations"
                              label="Enable Animations"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Enable smooth animations and transitions
                            </Text>
                          </div>

                          <div>
                            <Form.Item
                              name="enableNotifications"
                              label="Enable Notifications"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Show system notifications for important events
                            </Text>
                          </div>

                          <div>
                            <Form.Item
                              name="defaultContainerSize"
                              label="Default Container Size (MB)"
                              help="Default size for new containers"
                            >
                              <Select style={{ width: '100%' }}>
                                <Option value={100}>100 MB</Option>
                                <Option value={500}>500 MB</Option>
                                <Option value={1024}>1 GB</Option>
                                <Option value={5120}>5 GB</Option>
                                <Option value={10240}>10 GB</Option>
                              </Select>
                            </Form.Item>
                          </div>
                        </Space>
                      </Col>
                    </Row>
                  </motion.div>
                ),
              },
              {
                key: 'advanced',
                label: (
                  <span>
                    <FolderOutlined />
                    Advanced
                  </span>
                ),
                children: (
                  <motion.div
                    initial={{ opacity: 0, y: 20 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.3 }}
                  >
                    <Row gutter={[24, 24]}>
                      <Col span={24}>
                        <Space direction="vertical" className="w-full" size="large">
                          <div>
                            <Form.Item
                              name="logLevel"
                              label="Log Level"
                              help="Verbosity of application logs"
                            >
                              <Select style={{ width: '100%' }}>
                                <Option value="ERROR">Error</Option>
                                <Option value="WARN">Warning</Option>
                                <Option value="INFO">Info</Option>
                                <Option value="DEBUG">Debug</Option>
                                <Option value="TRACE">Trace</Option>
                              </Select>
                            </Form.Item>
                          </div>

                          <div>
                            <Form.Item
                              name="enableDebugMode"
                              label="Debug Mode"
                              valuePropName="checked"
                            >
                              <Switch
                                checkedChildren="ON"
                                unCheckedChildren="OFF"
                              />
                            </Form.Item>
                            <Text type="secondary" className="text-sm">
                              Enable debug mode for troubleshooting
                            </Text>
                          </div>

                          <div>
                            <Form.Item
                              name="backupLocation"
                              label="Backup Location"
                              help="Directory for automatic backups"
                            >
                              <Input
                                placeholder="Select backup directory..."
                                suffix={
                                  <Button
                                    type="text"
                                    icon={<FolderOutlined />}
                                    size="small"
                                    onClick={() => {
                                      // This would open a directory picker
                                      message.info('Directory picker would open')
                                    }}
                                  />
                                }
                              />
                            </Form.Item>
                          </div>
                        </Space>
                      </Col>
                    </Row>
                  </motion.div>
                ),
              },
            ]}
          />
        </Form>
      </Card>

      {/* Warning Alert */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5, delay: 0.4 }}
      >
        <Alert
          message="Important Security Notice"
          description="Changes to security settings will take effect after restarting the application. Make sure to test your settings with non-critical containers first."
          type="warning"
          showIcon
          className="mt-6"
        />
      </motion.div>
    </motion.div>
  )
}

export default Settings
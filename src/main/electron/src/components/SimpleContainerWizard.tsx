import { useState } from 'react'
import {
  Modal,
  Steps,
  Form,
  Input,
  Button,
  Space,
  Card,
  Row,
  Col,
  Progress,
  Alert,
  InputNumber,
  message,
} from 'antd'
import {
  InfoCircleOutlined,
  LockOutlined,
  CheckCircleOutlined,
  LoadingOutlined,
} from '@ant-design/icons'
import { motion, AnimatePresence } from 'framer-motion'
import { containerAPI, type ContainerConfig } from '../services/api'

interface SimpleContainerWizardProps {
  visible: boolean
  onClose: () => void
  onSuccess: () => void
}

interface SimpleFormData {
  name: string
  size: number
  password: string
  confirmPassword: string
}

function SimpleContainerWizard({ visible, onClose, onSuccess }: SimpleContainerWizardProps) {
  const [currentStep, setCurrentStep] = useState(0)
  const [form] = Form.useForm<SimpleFormData>()
  const [loading, setLoading] = useState(false)
  const [progress, setProgress] = useState(0)

  const steps = [
    {
      title: 'Basic Settings',
      icon: <InfoCircleOutlined />,
      description: 'Container name and size',
    },
    {
      title: 'Security',
      icon: <LockOutlined />,
      description: 'Set password',
    },
    {
      title: 'Create',
      icon: <CheckCircleOutlined />,
      description: 'Create container',
    },
  ]

  const handleNext = async () => {
    try {
      await form.validateFields()
      setCurrentStep(prev => prev + 1)
    } catch (error) {
      console.error('Validation failed:', error)
    }
  }

  const handlePrevious = () => {
    setCurrentStep(prev => prev - 1)
  }

  const handleCreate = async () => {
    try {
      setLoading(true)
      setProgress(0)
      
      const values = await form.validateFields()
      
      // Simulate progress
      const progressInterval = setInterval(() => {
        setProgress(prev => {
          if (prev >= 90) {
            clearInterval(progressInterval)
            return 100
          }
          return prev + 10
        })
      }, 200)
      
      const config: ContainerConfig = {
        name: values.name,
        size: values.size * 1024 * 1024, // Convert MB to bytes
        password: values.password,
        encryptionAlgorithm: 'AES-256-GCM',
        steganographic: false,
      }
      
      // Wait for progress to complete
      await new Promise(resolve => setTimeout(resolve, 2000))
      
      await containerAPI.createContainer(config)
      
      message.success('Container created successfully!')
      onSuccess()
      handleClose()
      
    } catch (error) {
      console.error('Failed to create container:', error)
      message.error('Failed to create container')
    } finally {
      setLoading(false)
      setProgress(0)
    }
  }

  const handleClose = () => {
    form.resetFields()
    setCurrentStep(0)
    setProgress(0)
    setLoading(false)
    onClose()
  }

  const renderBasicSettings = () => (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3 }}
    >
      <Card title="Basic Container Settings">
        <Row gutter={[24, 24]}>
          <Col span={24}>
            <Form.Item
              name="name"
              label="Container Name"
              rules={[
                { required: true, message: 'Please enter a container name' },
                { min: 3, message: 'Name must be at least 3 characters' },
                { max: 50, message: 'Name must be less than 50 characters' },
              ]}
            >
              <Input
                placeholder="Enter container name"
                size="large"
              />
            </Form.Item>
          </Col>
          
          <Col span={24}>
            <Form.Item
              name="size"
              label="Container Size (MB)"
              rules={[
                { required: true, message: 'Please specify container size' },
                { type: 'number', min: 10, message: 'Minimum size is 10 MB' },
                { type: 'number', max: 10240, message: 'Maximum size is 10 GB' },
              ]}
              initialValue={1024}
            >
              <InputNumber
                min={10}
                max={10240}
                style={{ width: '100%' }}
                size="large"
                formatter={value => `${value} MB`}
                parser={value => {
                  const parsed = value?.replace(' MB', '') || ''
                  return parseInt(parsed) || 0
                }}
              />
            </Form.Item>
          </Col>
        </Row>
      </Card>
    </motion.div>
  )

  const renderSecurity = () => (
    <motion.div
      initial={{ opacity: 0, x: 20 }}
      animate={{ opacity: 1, x: 0 }}
      exit={{ opacity: 0, x: -20 }}
      transition={{ duration: 0.3 }}
    >
      <Card title="Security Settings">
        <Row gutter={[24, 24]}>
          <Col span={12}>
            <Form.Item
              name="password"
              label="Master Password"
              rules={[
                { required: true, message: 'Please enter a password' },
                { min: 8, message: 'Password must be at least 8 characters' },
              ]}
            >
              <Input.Password
                placeholder="Enter master password"
                size="large"
              />
            </Form.Item>
          </Col>
          
          <Col span={12}>
            <Form.Item
              name="confirmPassword"
              label="Confirm Password"
              dependencies={['password']}
              rules={[
                { required: true, message: 'Please confirm your password' },
                ({ getFieldValue }) => ({
                  validator(_, value) {
                    if (!value || getFieldValue('password') === value) {
                      return Promise.resolve()
                    }
                    return Promise.reject(new Error('Passwords do not match'))
                  },
                }),
              ]}
            >
              <Input.Password
                placeholder="Confirm master password"
                size="large"
              />
            </Form.Item>
          </Col>
        </Row>
      </Card>
    </motion.div>
  )

  const renderCreate = () => {
    const values = form.getFieldsValue()
    
    return (
      <motion.div
        initial={{ opacity: 0, x: 20 }}
        animate={{ opacity: 1, x: 0 }}
        exit={{ opacity: 0, x: -20 }}
        transition={{ duration: 0.3 }}
      >
        {loading ? (
          <Card title="Creating Container" className="text-center">
            <div className="py-8">
              <LoadingOutlined className="text-4xl text-blue-500 mb-4" />
              <h4>Creating your container...</h4>
              <Progress
                percent={progress}
                strokeColor={{
                  '0%': '#0ea5e9',
                  '100%': '#06b6d4',
                }}
                className="mb-4"
              />
              <p className="text-gray-500">
                Please wait while your container is being created...
              </p>
            </div>
          </Card>
        ) : (
          <div className="space-y-6">
            <Card title="Container Summary">
              <Row gutter={[24, 16]}>
                <Col span={12}>
                  <div>
                    <strong>Name:</strong>
                    <div>{values.name}</div>
                  </div>
                </Col>
                <Col span={12}>
                  <div>
                    <strong>Size:</strong>
                    <div>{values.size} MB</div>
                  </div>
                </Col>
              </Row>
            </Card>
            
            <Alert
              message="Ready to Create Container"
              description="Please review the settings above. Click Create to proceed."
              type="success"
              showIcon
            />
          </div>
        )}
      </motion.div>
    )
  }

  const renderStepContent = () => {
    switch (currentStep) {
      case 0:
        return renderBasicSettings()
      case 1:
        return renderSecurity()
      case 2:
        return renderCreate()
      default:
        return null
    }
  }

  return (
    <Modal
      title="Create New Container"
      open={visible}
      onCancel={handleClose}
      width={700}
      footer={null}
      destroyOnClose
    >
      <div className="space-y-6">
        <Steps
          current={currentStep}
          items={steps}
          className="mb-8"
        />
        
        <Form
          form={form}
          layout="vertical"
          size="large"
          className="min-h-[300px]"
        >
          <AnimatePresence mode="wait">
            {renderStepContent()}
          </AnimatePresence>
        </Form>
        
        <div className="flex justify-between pt-4 border-t">
          <Button
            size="large"
            onClick={handleClose}
            disabled={loading}
          >
            Cancel
          </Button>
          
          <Space>
            {currentStep > 0 && (
              <Button
                size="large"
                onClick={handlePrevious}
                disabled={loading}
              >
                Previous
              </Button>
            )}
            
            {currentStep < steps.length - 1 ? (
              <Button
                type="primary"
                size="large"
                onClick={handleNext}
                disabled={loading}
              >
                Next
              </Button>
            ) : (
              <Button
                type="primary"
                size="large"
                onClick={handleCreate}
                loading={loading}
                icon={<CheckCircleOutlined />}
              >
                Create Container
              </Button>
            )}
          </Space>
        </div>
      </div>
    </Modal>
  )
}

export default SimpleContainerWizard
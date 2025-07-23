import { useEffect, useState } from 'react'
import { Card, Row, Col, Statistic, Progress, Button, Space, Alert } from 'antd'
import {
  FolderOutlined,
  LockOutlined,
  CloudUploadOutlined,
  SafetyOutlined,
  PlusOutlined,
  EyeOutlined,
} from '@ant-design/icons'
import { motion } from 'framer-motion'
import { useNavigate } from 'react-router-dom'
import { systemAPI, type SystemStatus } from '../services/api'
import SimpleContainerWizard from '../components/SimpleContainerWizard'

function Dashboard() {
  const navigate = useNavigate()
  const [systemStatus, setSystemStatus] = useState<SystemStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [wizardVisible, setWizardVisible] = useState(false)

  useEffect(() => {
    loadSystemStatus()
  }, [])

  const loadSystemStatus = async () => {
    try {
      setLoading(true)
      const status = await systemAPI.getStatus()
      setSystemStatus(status)
    } catch (error) {
      console.error('Failed to load system status:', error)
    } finally {
      setLoading(false)
    }
  }

  const formatBytes = (bytes: number) => {
    if (bytes === 0) return '0 Bytes'
    const k = 1024
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB']
    const i = Math.floor(Math.log(bytes) / Math.log(k))
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i]
  }

  const formatUptime = (seconds: number) => {
    const days = Math.floor(seconds / 86400)
    const hours = Math.floor((seconds % 86400) / 3600)
    const minutes = Math.floor((seconds % 3600) / 60)
    
    if (days > 0) return `${days}d ${hours}h ${minutes}m`
    if (hours > 0) return `${hours}h ${minutes}m`
    return `${minutes}m`
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
    <div className="space-y-6">
      {/* Welcome Section */}
      <motion.div
        initial={{ opacity: 0, y: -20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5 }}
      >
        <Card className="gradient-bg text-white border-0">
          <div className="flex items-center justify-between">
            <div>
              <h2 className="text-2xl font-bold text-white mb-2">
                Welcome to EFS
              </h2>
              <p className="text-white/80 text-lg">
                Secure your files with military-grade encryption
              </p>
            </div>
            <div className="flex space-x-3">
              <Button
                type="primary"
                size="large"
                icon={<PlusOutlined />}
                className="bg-white/20 border-white/30 hover:bg-white/30"
                onClick={() => setWizardVisible(true)}
              >
                New Container
              </Button>
              <Button
                size="large"
                icon={<EyeOutlined />}
                className="bg-white/10 border-white/20 text-white hover:bg-white/20"
                onClick={() => navigate('/containers')}
              >
                View All
              </Button>
            </div>
          </div>
        </Card>
      </motion.div>

      {/* System Status Alert */}
      {!loading && systemStatus && (
        <motion.div
          initial={{ opacity: 0, scale: 0.95 }}
          animate={{ opacity: 1, scale: 1 }}
          transition={{ duration: 0.3 }}
        >
          <Alert
            message="System Status: Operational"
            description={`EFS has been running for ${formatUptime(systemStatus.uptime)} with ${systemStatus.containersCount} containers managed.`}
            type="success"
            showIcon
            className="mb-6"
          />
        </motion.div>
      )}

      {/* Statistics Cards */}
      <Row gutter={[24, 24]}>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            custom={0}
            initial="hidden"
            animate="visible"
            variants={containerVariants}
          >
            <Card loading={loading} className="text-center">
              <Statistic
                title="Total Containers"
                value={systemStatus?.containersCount || 0}
                prefix={<FolderOutlined className="text-primary-500" />}
                valueStyle={{ color: '#0ea5e9' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            custom={1}
            initial="hidden"
            animate="visible"
            variants={containerVariants}
          >
            <Card loading={loading} className="text-center">
              <Statistic
                title="Mounted"
                value={systemStatus?.mountedCount || 0}
                prefix={<LockOutlined className="text-green-500" />}
                valueStyle={{ color: '#10b981' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            custom={2}
            initial="hidden"
            animate="visible"
            variants={containerVariants}
          >
            <Card loading={loading} className="text-center">
              <Statistic
                title="Total Size"
                value={formatBytes(systemStatus?.totalSize || 0)}
                prefix={<CloudUploadOutlined className="text-blue-500" />}
                valueStyle={{ color: '#3b82f6' }}
              />
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} sm={12} lg={6}>
          <motion.div
            custom={3}
            initial="hidden"
            animate="visible"
            variants={containerVariants}
          >
            <Card loading={loading} className="text-center">
              <Statistic
                title="Security Level"
                value="High"
                prefix={<SafetyOutlined className="text-orange-500" />}
                valueStyle={{ color: '#f59e0b' }}
              />
            </Card>
          </motion.div>
        </Col>
      </Row>

      {/* Storage Usage */}
      <Row gutter={[24, 24]}>
        <Col xs={24} lg={12}>
          <motion.div
            initial={{ opacity: 0, x: -20 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ duration: 0.5, delay: 0.4 }}
          >
            <Card title="Storage Usage" loading={loading}>
              <div className="space-y-4">
                <div>
                  <div className="flex justify-between mb-2">
                    <span>Used Space</span>
                    <span>{formatBytes(systemStatus?.totalSize || 0)}</span>
                  </div>
                  <Progress
                    percent={systemStatus ? Math.round((systemStatus.totalSize / (systemStatus.totalSize + systemStatus.availableSpace)) * 100) : 0}
                    strokeColor={{
                      '0%': '#0ea5e9',
                      '100%': '#06b6d4',
                    }}
                  />
                </div>
                <div className="text-sm text-gray-500 dark:text-gray-400">
                  Available: {formatBytes(systemStatus?.availableSpace || 0)}
                </div>
              </div>
            </Card>
          </motion.div>
        </Col>

        <Col xs={24} lg={12}>
          <motion.div
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ duration: 0.5, delay: 0.5 }}
          >
            <Card title="Quick Actions">
              <Space direction="vertical" className="w-full" size="middle">
                <Button
                  type="primary"
                  block
                  icon={<PlusOutlined />}
                  size="large"
                  onClick={() => setWizardVisible(true)}
                >
                  Create New Container
                </Button>
                <Button
                  block
                  icon={<FolderOutlined />}
                  size="large"
                  onClick={() => navigate('/containers')}
                >
                  Open Existing Container
                </Button>
                <Button
                  block
                  icon={<SafetyOutlined />}
                  size="large"
                  onClick={() => navigate('/containers')}
                >
                  Run Security Check
                </Button>
              </Space>
            </Card>
          </motion.div>
        </Col>
      </Row>

      {/* Container Creation Wizard */}
      <SimpleContainerWizard
        visible={wizardVisible}
        onClose={() => setWizardVisible(false)}
        onSuccess={() => {
          loadSystemStatus()
        }}
      />
    </div>
  )
}

export default Dashboard
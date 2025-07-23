import React, { useState, useEffect } from 'react'
import {
  Card,
  Button,
  Typography,
  Space,
  Statistic,
  Row,
  Col,
  message,
  Spin,
  Alert,
} from 'antd'
import {
  ShareAltOutlined,
  PlusOutlined,
  UserOutlined,
  CheckCircleOutlined,
  ClockCircleOutlined,
  ReloadOutlined,
} from '@ant-design/icons'
import { motion } from 'framer-motion'
import {
  SharedContainer,
  ReceivedShare,
  ShareConfig,
  Container,
  sharingAPI,
  containerAPI,
} from '../services/api'
import SharedContainersList from '../components/SharedContainersList'
import ShareDialog from '../components/ShareDialog'
import AuditTrail from '../components/AuditTrail'

const { Title, Text } = Typography

const Sharing: React.FC = () => {
  const [sharedContainers, setSharedContainers] = useState<SharedContainer[]>([])
  const [receivedShares, setReceivedShares] = useState<ReceivedShare[]>([])
  const [availableContainers, setAvailableContainers] = useState<Container[]>([])
  const [selectedContainer, setSelectedContainer] = useState<Container | null>(null)
  const [shareDialogVisible, setShareDialogVisible] = useState(false)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const [stats, setStats] = useState({
    totalShared: 0,
    totalReceived: 0,
    activeShares: 0,
    pendingShares: 0,
  })

  // Load all sharing data
  const loadSharingData = async (showLoading = true) => {
    if (showLoading) setLoading(true)
    setRefreshing(true)

    try {
      const [
        myShares,
        received,
        containers,
        sharingStats,
      ] = await Promise.all([
        sharingAPI.getMyShares(),
        sharingAPI.getReceivedShares(),
        sharingAPI.getShareableContainers(),
        sharingAPI.getSharingStats(),
      ])

      setSharedContainers(myShares)
      setReceivedShares(received)
      setAvailableContainers(containers)
      setStats(sharingStats)
    } catch (error: any) {
      console.error('Failed to load sharing data:', error)
      message.error('Failed to load sharing data')
    } finally {
      setLoading(false)
      setRefreshing(false)
    }
  }

  // Load data on component mount
  useEffect(() => {
    loadSharingData()
  }, [])

  // Handle container sharing
  const handleShare = async (shareConfig: ShareConfig): Promise<SharedContainer> => {
    if (!selectedContainer) {
      throw new Error('No container selected')
    }

    try {
      const result = await sharingAPI.shareWithUser(selectedContainer.id, shareConfig.recipientUsername, shareConfig)
      await loadSharingData(false) // Refresh data without showing loading
      return result
    } catch (error: any) {
      console.error('Failed to share container:', error)
      throw new Error(error.response?.data?.message || 'Failed to share container')
    }
  }

  // Handle share revocation
  const handleRevokeShare = async (shareId: string) => {
    try {
      await sharingAPI.revokeUserShare(shareId)
      await loadSharingData(false)
    } catch (error: any) {
      console.error('Failed to revoke share:', error)
      throw new Error(error.response?.data?.message || 'Failed to revoke share')
    }
  }

  // Handle share acceptance
  const handleAcceptShare = async (shareId: string) => {
    try {
      await sharingAPI.acceptShare(shareId)
      await loadSharingData(false)
    } catch (error: any) {
      console.error('Failed to accept share:', error)
      throw new Error(error.response?.data?.message || 'Failed to accept share')
    }
  }

  // Handle share decline
  const handleDeclineShare = async (shareId: string) => {
    try {
      await sharingAPI.declineShare(shareId)
      await loadSharingData(false)
    } catch (error: any) {
      console.error('Failed to decline share:', error)
      throw new Error(error.response?.data?.message || 'Failed to decline share')
    }
  }

  // Open share dialog for container
  const openShareDialog = (container: Container) => {
    setSelectedContainer(container)
    setShareDialogVisible(true)
  }

  // Close share dialog
  const closeShareDialog = () => {
    setSelectedContainer(null)
    setShareDialogVisible(false)
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <Spin size="large" />
      </div>
    )
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
          <Title level={2} className="mb-2 flex items-center space-x-3">
            <ShareAltOutlined className="text-primary-500" />
            <span>Container Sharing</span>
          </Title>
          <Text type="secondary">
            Share your encrypted containers with other users securely
          </Text>
        </div>
        <Button
          type="primary"
          icon={<ReloadOutlined />}
          onClick={() => loadSharingData(false)}
          loading={refreshing}
          className="bg-gradient-to-r from-primary-500 to-primary-600"
        >
          Refresh
        </Button>
      </div>

      {/* Statistics Cards */}
      <Row gutter={[16, 16]}>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            whileHover={{ scale: 1.02 }}
            transition={{ duration: 0.2 }}
          >
            <Card className="glass-card text-center">
              <Statistic
                title="Containers Shared"
                value={stats.totalShared}
                prefix={<ShareAltOutlined className="text-blue-500" />}
                valueStyle={{ color: '#3b82f6' }}
              />
            </Card>
          </motion.div>
        </Col>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            whileHover={{ scale: 1.02 }}
            transition={{ duration: 0.2 }}
          >
            <Card className="glass-card text-center">
              <Statistic
                title="Shares Received"
                value={stats.totalReceived}
                prefix={<UserOutlined className="text-green-500" />}
                valueStyle={{ color: '#10b981' }}
              />
            </Card>
          </motion.div>
        </Col>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            whileHover={{ scale: 1.02 }}
            transition={{ duration: 0.2 }}
          >
            <Card className="glass-card text-center">
              <Statistic
                title="Active Shares"
                value={stats.activeShares}
                prefix={<CheckCircleOutlined className="text-emerald-500" />}
                valueStyle={{ color: '#059669' }}
              />
            </Card>
          </motion.div>
        </Col>
        <Col xs={24} sm={12} lg={6}>
          <motion.div
            whileHover={{ scale: 1.02 }}
            transition={{ duration: 0.2 }}
          >
            <Card className="glass-card text-center">
              <Statistic
                title="Pending Shares"
                value={stats.pendingShares}
                prefix={<ClockCircleOutlined className="text-orange-500" />}
                valueStyle={{ color: '#f59e0b' }}
              />
            </Card>
          </motion.div>
        </Col>
      </Row>

      {/* Quick Share Section */}
      {availableContainers.length > 0 && (
        <Card className="glass-card">
          <div className="flex items-center justify-between mb-4">
            <Title level={4} className="mb-0">
              Quick Share
            </Title>
            <Text type="secondary">
              Select a container to share with other users
            </Text>
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {availableContainers.slice(0, 6).map(container => (
              <motion.div
                key={container.id}
                whileHover={{ scale: 1.02 }}
                transition={{ duration: 0.2 }}
              >
                <Card
                  size="small"
                  className="cursor-pointer hover:shadow-lg transition-all duration-300"
                  onClick={() => openShareDialog(container)}
                >
                  <div className="flex items-center justify-between">
                    <div>
                      <Text strong>{container.name}</Text>
                      <br />
                      <Text type="secondary" className="text-xs">
                        {(container.size / (1024 * 1024 * 1024)).toFixed(2)} GB
                      </Text>
                    </div>
                    <Button
                      type="primary"
                      size="small"
                      icon={<PlusOutlined />}
                      onClick={(e) => {
                        e.stopPropagation()
                        openShareDialog(container)
                      }}
                    >
                      Share
                    </Button>
                  </div>
                </Card>
              </motion.div>
            ))}
          </div>
        </Card>
      )}

      {/* No containers available */}
      {availableContainers.length === 0 && (
        <Alert
          message="No Containers Available"
          description="You need to create and mount containers before you can share them with other users."
          type="info"
          showIcon
          className="glass-card"
        />
      )}

      {/* Shared Containers List */}
      <SharedContainersList
        sharedContainers={sharedContainers}
        receivedShares={receivedShares}
        onRevokeShare={handleRevokeShare}
        onAcceptShare={handleAcceptShare}
        onDeclineShare={handleDeclineShare}
        onRefresh={() => loadSharingData(false)}
        loading={refreshing}
      />

      {/* Audit Trail */}
      <AuditTrail
        title="Sharing Activity Log"
        height={300}
      />

      {/* Share Dialog */}
      <ShareDialog
        visible={shareDialogVisible}
        container={selectedContainer}
        onShare={handleShare}
        onCancel={closeShareDialog}
      />
    </motion.div>
  )
}

export default Sharing
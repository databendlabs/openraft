(ns jepsen.openraft.nemesis.membership
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.await :as await]
            [jepsen.openraft [client :as client]
             [checker :as openraft-checker]
             [cluster :as cluster]
             [db :as openraft-db]]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]))

(def change-seconds 10)
(def minimum-voters 3)
(def learner-wait-timeout-ms 60000)

(def ^:private membership-error-arm-limit 8)
(def ^:private membership-error-arm-name-limit 128)

(defn- select-node [test candidates]
  (->> (:nodes test)
       (filter (set candidates))
       random/shuffle
       first))

(defn- voter-ids [test voters]
  (->> (:nodes test)
       (filter (set voters))
       (mapv client/node-host)))

(defn- request-leader! [test leader-endpoint request]
  (client/with-leader!
    leader-endpoint
    (mapv #(client/api-endpoint test %) (:nodes test))
    request))

(defn- ambiguous-request-error? [e]
  (let [{:keys [kind status]} (ex-data e)]
    (or (#{:request-timeout
           :transport-error
           :invalid-json
           :invalid-response} kind)
        (and (= :http-error kind)
             (or (nil? status)
                 (<= 500 status 599))))))

(defn- leader-routing-error? [e]
  (let [{:keys [kind error]} (ex-data e)]
    (or (= :unreachable kind)
        (contains? error :ForwardToLeader))))

(defn- change-membership-error [e]
  (let [error (:error (ex-data e))]
    (when (and (map? error)
               (contains? error :ChangeMembershipError))
      {:payload (:ChangeMembershipError error)})))

(defn- bounded-arm-name [arm]
  (let [arm-name (if (keyword? arm) (name arm) (str arm))]
    (subs arm-name 0 (min membership-error-arm-name-limit
                          (count arm-name)))))

(defn- malformed-membership-error-evidence [e]
  (when-let [{:keys [payload]} (change-membership-error e)]
    (when-not (and (map? payload) (= 1 (count payload)))
      {:error-shape (if (map? payload) :map :not-map)
       :error-arm-count (when (map? payload) (count payload))
       :error-arm-names (if (map? payload)
                          (->> (keys payload)
                               (take membership-error-arm-limit)
                               (map bounded-arm-name)
                               sort
                               vec)
                          [])})))

(defn- membership-error-variant [e]
  (let [payload (:payload (change-membership-error e))]
    (when (and (map? payload) (= 1 (count payload)))
      (ffirst payload))))

(defn- membership-change-in-progress? [e]
  (= :InProgress (membership-error-variant e)))

(defn- unexpected-membership-error-outcome [e]
  (when-let [evidence (malformed-membership-error-evidence e)]
    (outcome/indeterminate :unexpected-sut-response evidence)))

(defn- await-reachable-learner! [test node]
  (cluster/await-node-metrics! test node learner-wait-timeout-ms))

(defn- request-membership-change!
  [test leader-endpoint target-voters]
  (try
    (request-leader!
     test
     leader-endpoint
     #(client/change-membership! % (voter-ids test target-voters)))
    :completed
    (catch Exception e
      (let [unexpected (unexpected-membership-error-outcome e)]
        (cond
          (leader-routing-error? e)
          :no-supported-leader

          unexpected
          unexpected

          (ambiguous-request-error? e)
          (do
            (info "OpenRaft membership change result is indeterminate"
                  {:target-voters target-voters
                   :error (ex-message e)})
            :indeterminate)

          (membership-change-in-progress? e)
          (do
            (info "OpenRaft membership change is already in progress"
                  {:target-voters target-voters
                   :error (ex-message e)})
            :in-progress)

          :else
          (throw e))))))

(def ^:private restoration-outcome-kind ::restoration-outcome)

(defn- closed-outcome? [value]
  (contains? #{:installed :skipped :indeterminate} (:status value)))

(defn- restoration-context [pending]
  (atom
   (cond-> (merge {:attempted? (= :indeterminate (:status pending))}
                  (select-keys pending
                               [:stage
                                :error-variant
                                :error-shape
                                :error-arm-count
                                :error-arm-names]))
     (= :indeterminate (:status pending))
     (assoc :reason (or (:reason pending) :request-result-unknown))
     (= :in-progress (:status pending))
     (assoc :skip-reason :membership-change-in-progress)
     (#{:learner-unreachable :no-supported-leader :node-starting}
      (:status pending))
     (assoc :skip-reason (:status pending)))))

(defn- return-restoration-outcome! [value]
  (throw (ex-info "OpenRaft membership restoration has a modeled outcome"
                  {:kind restoration-outcome-kind
                   :outcome value})))

(defn- note-restoration-request!
  [context stage result details]
  (let [status (if (map? result) (:status result) result)
        evidence (when (map? result)
                   (select-keys result
                                [:reason
                                 :error-variant
                                 :error-shape
                                 :error-arm-count
                                 :error-arm-names]))]
    (case status
      :completed
      (swap! context assoc
             :attempted? true
             :stage stage)

      :indeterminate
      (swap! context
             #(merge %
                     evidence
                     {:attempted? true
                      :reason (or (:reason evidence)
                                  :request-result-unknown)
                      :stage stage}))

      :in-progress
      (swap! context assoc
             :skip-reason :membership-change-in-progress
             :stage stage)

      :no-supported-leader
      (let [{:keys [attempted? reason]
             prior-stage :stage} @context
            details (assoc details :stage (or prior-stage stage))]
        (return-restoration-outcome!
         (if attempted?
           (outcome/indeterminate
            (or reason :membership-recovery-unconfirmed)
            details)
           (outcome/skipped :no-supported-leader details))))

      nil))
  result)

(defn- restoration-timeout-outcome [context e]
  (let [{:keys [attempted? reason skip-reason] :as state} @context
        details (merge {:condition (:condition (ex-data e))
                        :message (ex-message e)}
                       (select-keys state
                                    [:stage
                                     :error-variant
                                     :error-shape
                                     :error-arm-count
                                     :error-arm-names]))]
    (if attempted?
      (outcome/indeterminate
       (or reason :membership-recovery-unconfirmed)
       details)
      (outcome/skipped (or skip-reason :membership-not-ready)
                       details))))

(defn- request-membership-change-when-ready!
  [test leader-endpoint target-voters context]
  (let [waiting-for-stable? (atom false)]
    (await/until!
     :membership-change-request-ready
     #(let [waiting? @waiting-for-stable?
            status (when waiting? (cluster/membership-status test))]
        (cond
          (and waiting? (not (:stable? status)))
          (await/retry! :membership-change-request-ready
                        {:target-voters target-voters
                         :status status})

          (and waiting? (= target-voters (:voters status)))
          status

          :else
          (do
            (reset! waiting-for-stable? false)
            (let [result (request-membership-change! test
                                                     leader-endpoint
                                                     target-voters)]
              (note-restoration-request! context
                                         :change-membership
                                         result
                                         {:target-voters target-voters})
              (cond
                (closed-outcome? result)
                (return-restoration-outcome!
                 (assoc result :stage :change-membership))

                (= :in-progress result)
                (do
                  (reset! waiting-for-stable? true)
                  (await/retry! :membership-change-request-ready
                                {:target-voters target-voters}))

                :else
                result)))))
     {:log-message "Waiting to retry an OpenRaft membership change"
      :timeout learner-wait-timeout-ms})))

(defn- complete-joint-membership!
  [test {:keys [leader effective-voter-configs]} context]
  (let [target-voters (peek effective-voter-configs)
        leader-endpoint (atom (client/api-endpoint test leader))
        result (request-membership-change-when-ready! test
                                                      leader-endpoint
                                                      target-voters
                                                      context)]
    (info "Completing an existing joint OpenRaft membership"
          {:configs effective-voter-configs
           :target-voters target-voters})
    (if (map? result)
      result
      (cluster/await-stable-membership! test target-voters))))

(defn- stable-membership!
  ([test]
   (stable-membership! test (restoration-context nil)))
  ([test context]
   (let [{:keys [stable?] :as status}
         (cluster/await-committed-membership! test)]
     (if stable?
       status
       (complete-joint-membership! test status context)))))

(defn- change-membership-and-await!
  [test leader-endpoint target-voters context]
  (let [result (request-membership-change-when-ready! test
                                                      leader-endpoint
                                                      target-voters
                                                      context)]
    (if (map? result)
      result
      (try
        (cluster/await-stable-membership! test target-voters)
        (catch Exception e
          (when-not (await/condition-timeout? e :stable-membership)
            (throw e))
          (let [status (stable-membership! test context)]
            (if (= target-voters (:voters status))
              status
              (throw e))))))))

(defn- await-observed-learner! [test node]
  (cluster/await-observed-learner! test node learner-wait-timeout-ms))

(defn- validate-add-learner-request!
  [test node node-id api-addr raft-addr]
  (let [configured? (contains? (set (:nodes test)) node)
        expected {:node-id (client/node-host node)
                  :api-addr (client/api-endpoint test node)
                  :raft-addr (client/raft-addr test node)}
        actual {:node-id node-id
                :api-addr api-addr
                :raft-addr raft-addr}]
    (when-not (and configured? (= expected actual))
      (throw (ex-info "Invalid OpenRaft add-learner request"
                      {:kind :invalid-add-learner-request
                       :node node
                       :configured? configured?
                       :expected expected
                       :actual actual})))))

(defn- unexpected-add-learner-error-variant [e]
  (let [variant (membership-error-variant e)]
    (when (#{:LearnerNotFound :EmptyMembership} variant)
      variant)))

(defn- request-add-learner!
  [test leader-endpoint node node-id api-addr raft-addr]
  (validate-add-learner-request! test node node-id api-addr raft-addr)
  (try
    (request-leader!
     test
     leader-endpoint
     #(client/add-learner! % node-id api-addr raft-addr))
    :completed
    (catch Exception e
      (let [unexpected (unexpected-membership-error-outcome e)]
        (cond
          (leader-routing-error? e)
          :no-supported-leader

          unexpected
          unexpected

          (ambiguous-request-error? e)
          (do
            (info "OpenRaft learner addition result is indeterminate"
                  {:node node
                   :error (ex-message e)})
            :indeterminate)

          (membership-change-in-progress? e)
          (do
            (info "OpenRaft learner addition found a membership change in progress"
                  {:node node
                   :error (ex-message e)})
            :in-progress)

          (unexpected-add-learner-error-variant e)
          (let [variant (unexpected-add-learner-error-variant e)]
            (info "OpenRaft learner addition returned an unexpected error"
                  {:node node
                   :error-variant variant})
            (outcome/indeterminate :unexpected-sut-response
                                   {:error-variant variant}))

          :else
          (throw e))))))

(defn- request-add-learner-when-ready!
  [test leader-endpoint node node-id api-addr raft-addr context]
  (let [waiting-for-stable? (atom false)]
    (await/until!
     :add-learner-request-ready
     #(let [waiting? @waiting-for-stable?
            status (when waiting? (cluster/membership-status test))]
        (cond
          (and waiting? (not (:stable? status)))
          (await/retry! :add-learner-request-ready
                        {:node node
                         :status status})

          (and waiting?
               (or (contains? (:learners status) node)
                   (contains? (:voters status) node)))
          status

          :else
          (do
            (reset! waiting-for-stable? false)
            (let [result (request-add-learner! test
                                               leader-endpoint
                                               node
                                               node-id
                                               api-addr
                                               raft-addr)]
              (note-restoration-request! context
                                         :add-learner
                                         result
                                         {:node node})
              (cond
                (closed-outcome? result)
                (return-restoration-outcome! (assoc result
                                                    :stage
                                                    :add-learner))

                (= :in-progress result)
                (do
                  (reset! waiting-for-stable? true)
                  (await/retry! :add-learner-request-ready {:node node}))

                :else
                result)))))
     {:log-message (str "Waiting to retry adding OpenRaft learner " node)
      :timeout learner-wait-timeout-ms})))

(defn- add-learner-and-confirm!
  [test leader-endpoint node node-id api-addr raft-addr context]
  (let [result (request-add-learner-when-ready! test
                                                leader-endpoint
                                                node
                                                node-id
                                                api-addr
                                                raft-addr
                                                context)]
    (if (= :indeterminate result)
      (await-observed-learner! test node)
      result)))

(defn- grow! [database test context]
  (let [{:keys [leader voters learners non-members metrics]}
        (stable-membership! test context)
        source (if (seq learners) :learner :non-member)
        node (select-node test
                          (if (= :learner source)
                            learners
                            non-members))]
    (if-not node
      :membership-full
      (let [leader-endpoint (atom (client/api-endpoint test leader))
            {:keys [node-id api-addr raft-addr]}
            (cluster/node-info test node)
            target-voters (conj voters node)]
        (when (= :non-member source)
          (openraft-db/start-empty-node! database test node))
        (when (and (= :learner source)
                   (not (contains? metrics node)))
          (await-reachable-learner! test node))

        (info "Adding OpenRaft learner"
              {:node node
               :source source
               :leader leader})
        (let [learner-status
              (add-learner-and-confirm!
               test
               leader-endpoint
               node
               node-id
               api-addr
               raft-addr
               context)]

          (info "Growing OpenRaft membership"
                {:node node
                 :before voters
                 :after target-voters})

          (let [final-status
                (if (and (map? learner-status)
                         (= target-voters (:voters learner-status)))
                  learner-status
                  (change-membership-and-await!
                   test
                   leader-endpoint
                   target-voters
                   context))]
            {:node node
             :source source
             :leader (:leader final-status)
             :before voters
             :after (:voters final-status)}))))))

(defn- shrink-candidates [{:keys [voters metrics]}]
  (let [reachable (set (keys metrics))]
    (filter
     (fn [node]
       (let [target-voters (disj voters node)]
         (quorum/quorum? [target-voters] reachable)))
     voters)))

(defn- complete-pending-change!
  [pending-change pending leader]
  (reset! pending-change nil)
  (-> pending
      (dissoc :target
              :status
              :reason
              :stage
              :error-variant
              :error-shape
              :error-arm-count
              :error-arm-names)
      (assoc :leader leader
             :after (:target pending))))

(defn- retained-status-reason [status]
  (case status
    :indeterminate :request-result-unknown
    :in-progress :membership-change-in-progress
    :learner-unreachable :learner-unreachable
    :no-supported-leader :no-supported-leader
    :node-starting :node-starting
    nil))

(defn- retain-pending-change!
  [pending-change pending result]
  (let [current @pending-change
        status (if (map? result) (:status result) result)
        reason (if (map? result)
                 (:reason result)
                 (retained-status-reason status))
        evidence (when (map? result)
                   (select-keys result
                                [:error-variant
                                 :error-shape
                                 :error-arm-count
                                 :error-arm-names]))
        retained (if (= :indeterminate (:status current))
                   current
                   (cond-> (merge pending evidence {:status status})
                     reason (assoc :reason reason)))]
    (reset! pending-change retained)
    retained))

(defn- request-pending-membership-change!
  [pending-change test status pending]
  (let [result (request-membership-change!
                test
                (atom (client/api-endpoint test (:leader status)))
                (:target pending))]
    (if (= :completed result)
      (complete-pending-change! pending-change pending (:leader status))
      (retain-pending-change! pending-change pending result))))

(defn- request-grow-change!
  [pending-change test status pending]
  (let [{:keys [node target]} pending
        {:keys [node-id api-addr raft-addr]} (cluster/node-info test node)
        leader-endpoint (atom (client/api-endpoint test (:leader status)))]
    (info "Adding OpenRaft learner"
          {:node node
           :source (:source pending)
           :leader (:leader status)})
    (let [learner-result (request-add-learner!
                          test
                          leader-endpoint
                          node
                          node-id
                          api-addr
                          raft-addr)]
      (if-not (= :completed learner-result)
        (retain-pending-change! pending-change
                                (assoc pending :stage :add-learner)
                                learner-result)
        (let [current @pending-change
              pending (cond-> (assoc pending
                                     :stage :change-membership)
                        (and (= :indeterminate (:status current))
                             (= :add-learner (:stage current)))
                        (dissoc :status
                                :reason
                                :error-variant
                                :error-shape
                                :error-arm-count
                                :error-arm-names))]
          (when current
            (reset! pending-change pending))
          (info "Growing OpenRaft membership"
                {:node node
                 :before (:before pending)
                 :after target})
          (request-pending-membership-change!
           pending-change
           test
           status
           pending))))))

(defn- request-grow! [database pending-change test status]
  (let [{:keys [leader voters learners non-members metrics]} status
        source (if (seq learners) :learner :non-member)
        node (select-node test
                          (if (= :learner source)
                            learners
                            non-members))]
    (if-not node
      :membership-full
      (let [pending {:change :grow
                     :node node
                     :source source
                     :leader leader
                     :before voters
                     :target (conj voters node)}]
        (cond
          (and (= :learner source)
               (not (contains? metrics node)))
          (retain-pending-change!
           pending-change
           pending
           :learner-unreachable)

          (and (= :non-member source)
               (not (contains? metrics node)))
          (do
            (openraft-db/start-empty-node-without-wait!
             database
             test
             node)
            (retain-pending-change!
             pending-change
             pending
             :node-starting))

          :else
          (request-grow-change!
           pending-change
           test
           status
           pending))))))

(defn- resolve-pending-grow!
  [pending-change test status]
  (let [{:keys [node target] :as pending} @pending-change]
    (cond
      (and (:stable? status)
           (= target (:voters status)))
      (complete-pending-change! pending-change pending (:leader status))

      (not (:stable? status))
      (request-pending-membership-change!
       pending-change
       test
       status
       pending)

      (contains? (:metrics status) node)
      (request-grow-change! pending-change test status pending)

      :else
      (assoc pending
             :status (if (contains? (:learners status) node)
                       :learner-unreachable
                       :node-starting)))))

(defn- request-shrink! [database pending-change test status]
  (let [{:keys [leader voters learners]} status]
    (cond
      (seq learners)
      :learner-pending

      (<= (count voters) minimum-voters)
      :minimum-membership

      :else
      (if-let [node (select-node test (shrink-candidates status))]
        (let [leader-endpoint (atom (client/api-endpoint test leader))
              target-voters (disj voters node)
              pending {:change :shrink
                       :node node
                       :leader leader
                       :before voters
                       :target target-voters}]
          (info "Shrinking OpenRaft membership"
                {:node node
                 :before voters
                 :after target-voters})
          (let [result (request-membership-change!
                        test
                        leader-endpoint
                        target-voters)]
            (if (= :completed result)
              (do
                (openraft-db/stop-and-wipe-node! database test node)
                (-> pending
                    (dissoc :target)
                    (assoc :after target-voters)))
              (retain-pending-change! pending-change
                                      (assoc pending
                                             :stage :change-membership)
                                      result))))
        :no-quorum-safe-shrink))))

(defn- resolve-pending-removal!
  [database pending-change test status]
  (let [{:keys [node target] :as pending} @pending-change
        completed? (and (:stable? status)
                        (= target (:voters status)))
        result (if completed?
                 :completed
                 (request-membership-change!
                  test
                  (atom (client/api-endpoint test (:leader status)))
                  target))]
    (if (= :completed result)
      (do
        (openraft-db/stop-and-wipe-node! database test node)
        (complete-pending-change!
         pending-change
         pending
         (:leader status)))
      (retain-pending-change! pending-change pending result))))

(defn- complete-joint-once! [test status]
  (let [target-voters (peek (:effective-voter-configs status))
        result (request-membership-change!
                test
                (atom (client/api-endpoint test (:leader status)))
                target-voters)]
    (merge (if (map? result) result {:status result})
           {:target target-voters
            :voter-configs (:effective-voter-configs status)})))

(defn- request-membership-operation!
  [database pending-change test operation]
  (if-let [status (cluster/membership-status test)]
    (cond
      @pending-change
      (case (:change @pending-change)
        :grow (resolve-pending-grow! pending-change test status)
        :shrink (resolve-pending-removal!
                 database
                 pending-change
                 test
                 status))

      (not (:stable? status))
      (complete-joint-once! test status)

      (= :grow operation)
      (request-grow! database pending-change test status)

      (= :shrink operation)
      (request-shrink! database pending-change test status))
    :no-supported-leader))

(def ^:private skipped-results
  #{:learner-pending
    :membership-full
    :minimum-membership
    :no-quorum-safe-shrink
    :no-supported-leader})

(def ^:private skipped-status-reasons
  {:in-progress :membership-change-in-progress
   :learner-unreachable :learner-unreachable
   :no-supported-leader :no-supported-leader
   :node-starting :node-starting})

(defn- normalize-membership-result [result]
  (cond
    (= :indeterminate result)
    (outcome/indeterminate :request-result-unknown {})

    (contains? skipped-results result)
    (outcome/skipped result {})

    (map? result)
    (let [status (:status result)
          details (dissoc result :status :reason)]
      (cond
        (= :completed status)
        (outcome/installed details)

        (= :indeterminate status)
        (outcome/indeterminate (or (:reason result)
                                   :request-result-unknown)
                               details)

        (contains? skipped-status-reasons status)
        (outcome/skipped (get skipped-status-reasons status) details)

        (contains? result :after)
        (outcome/installed (dissoc result :reason))

        :else
        (throw (ex-info "Unknown membership Nemesis result"
                        {:result result}))))

    :else
    (throw (ex-info "Unknown membership Nemesis result"
                    {:result result}))))

(defn- membership-operation-outcome [operation result]
  (let [normalized (normalize-membership-result result)
        actual-change (when (map? result) (:change result))
        indeterminate? (= :indeterminate (:status normalized))]
    (cond
      (and indeterminate?
           actual-change
           (not= operation actual-change))
      (outcome/indeterminate (:reason normalized)
                             {:pending-change normalized})

      (and indeterminate?
           (map? result)
           (contains? result :voter-configs))
      (outcome/indeterminate (:reason normalized)
                             {:existing-change normalized})

      (and actual-change (not= operation actual-change))
      (outcome/skipped
       :pending-membership-change
       {(if (= :installed (:status normalized))
          :resolved-change
          :pending-change) normalized})

      (and (map? result) (contains? result :voter-configs))
      (outcome/skipped :membership-change-in-progress
                       {:existing-change normalized})

      :else
      normalized)))

(defn- restoration-outcome [result]
  (if (closed-outcome? result)
    result
    (outcome/installed
     (cond-> result
       (:resolved-change result)
       (update :resolved-change outcome/installed)))))

(defn- complete-pending-removal-and-await!
  [database pending-change test context]
  (when-let [{:keys [node target] :as pending} @pending-change]
    (let [status (stable-membership! test context)
          final-status (if (= target (:voters status))
                         status
                         (change-membership-and-await!
                          test
                          (atom (client/api-endpoint test (:leader status)))
                          target
                          context))]
      (openraft-db/stop-and-wipe-node! database test node)
      (reset! pending-change nil)
      (-> pending
          (dissoc :target)
          (assoc :leader (:leader final-status)
                 :after target)))))

(defn- confirm-pending-grow-status!
  [pending-change status]
  (when-let [{:keys [change target] :as pending} @pending-change]
    (when (and (= :grow change)
               (= target (:voters status)))
      (reset! pending-change nil)
      (-> pending
          (dissoc :target
                  :status
                  :reason
                  :stage
                  :error-variant
                  :error-shape
                  :error-arm-count
                  :error-arm-names)
          (assoc :leader (:leader status)
                 :after target)))))

(defn- confirm-pending-grow!
  [pending-change test context]
  (let [status (stable-membership! test context)]
    (confirm-pending-grow-status! pending-change status)))

(defn- complete-pending-change-and-await!
  [database pending-change test context]
  (case (:change @pending-change)
    :grow (confirm-pending-grow! pending-change test context)
    :shrink (complete-pending-removal-and-await!
             database
             pending-change
             test
             context)
    nil nil))

(defn- restore-membership! [database pending-change test context]
  (let [resolved-change (complete-pending-change-and-await!
                         database
                         pending-change
                         test
                         context)
        target-voters (set (:nodes test))]
    (loop [status (stable-membership! test context)]
      (if (= target-voters (:voters status))
        (let [resolved-change (or resolved-change
                                  (confirm-pending-grow-status!
                                   pending-change
                                   status))]
          (cond-> {:leader (:leader status)
                   :voters (:voters status)}
            resolved-change
            (assoc :resolved-change resolved-change)))
        (let [result (grow! database test context)]
          (when (keyword? result)
            (return-restoration-outcome!
             (outcome/skipped result
                              {:status status
                               :target-voters target-voters})))
          (recur (stable-membership! test context)))))))

(defn- restore-membership-outcome!
  [database pending-change test]
  (let [context (restoration-context @pending-change)]
    (try
      (restoration-outcome
       (restore-membership! database pending-change test context))
      (catch Exception e
        (cond
          (= restoration-outcome-kind (:kind (ex-data e)))
          (:outcome (ex-data e))

          (some #(await/condition-timeout? e %)
                [:add-learner-request-ready
                 :committed-membership
                 :learner-observed
                 :membership-change-request-ready
                 :node-metrics
                 :stable-membership])
          (restoration-timeout-outcome context e)

          :else
          (throw e))))))

(defrecord MembershipNemesis [database pending-change]
  nemesis/Nemesis
  (setup! [_ _test]
    (MembershipNemesis. database (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :grow
      (let [result (request-membership-operation!
                    database
                    pending-change
                    test
                    :grow)]
        (assoc op :value (membership-operation-outcome :grow result)))

      :shrink
      (let [result (request-membership-operation!
                    database
                    pending-change
                    test
                    :shrink)]
        (assoc op :value (membership-operation-outcome :shrink result)))

      :restore-membership
      (assoc op
             :value (restore-membership-outcome!
                     database
                     pending-change
                     test))))

  (teardown! [_ _test])

  nemesis/Reflection
  (fs [_]
    #{:grow :shrink :restore-membership}))

(defn membership-nemesis [database]
  (MembershipNemesis. database (atom nil)))

(defn- membership-generator []
  (gen/phases
   {:type :info
    :f :shrink}
   {:type :info
    :f :grow}
   (gen/mix [(repeat {:type :info
                      :f :grow})
             (repeat {:type :info
                      :f :shrink})])))

(defn- membership-change-installed? [required-changes change]
  (and (= :installed (:status change))
       (required-changes (:change change))
       (:node change)
       (:leader change)
       (coll? (:before change))
       (coll? (:after change))
       (not= (:before change) (:after change))))

(defn- membership-restored? [expected-voters value]
  (and (= :installed (:status value))
       (:leader value)
       (= expected-voters (:voters value))))

(defn- recovery-installed? [value]
  (and (= :installed (:status value))
       (:leader value)))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ test history _opts]
      (let [required-changes #{:grow :shrink}
            expected-voters (set (:nodes test))
            membership-history (->> history
                                    (filter
                                     #(#{:grow
                                         :shrink
                                         :restore-membership} (:f %)))
                                    vec)
            errors (->> membership-history
                        (filter #(or (:error %)
                                     (:exception %)))
                        vec)
            observed-changes (->> membership-history
                                  (mapcat
                                   (fn [op]
                                     (let [value (:value op)]
                                       [value
                                        (:resolved-change value)])))
                                  (keep
                                   (fn [change]
                                     (when (membership-change-installed?
                                            required-changes
                                            change)
                                       (:change change))))
                                  set)
            missing-changes (remove observed-changes
                                    required-changes)
            final-operation (peek membership-history)
            restored? (and (= :restore-membership (:f final-operation))
                           (membership-restored?
                            expected-voters
                            (:value final-operation)))
            final-recovery (->> history
                                (filter #(= :await-recovery (:f %)))
                                last)
            recovered? (boolean
                        (recovery-installed? (:value final-recovery)))]
        {:valid? (and (empty? errors)
                      (empty? missing-changes)
                      restored?
                      recovered?)
         :observed-changes (vec (sort observed-changes))
         :missing-changes (vec (sort missing-changes))
         :restored? restored?
         :recovered? recovered?
         :error-count (count errors)
         :errors (vec (take 10 errors))}))))

(defn membership-package [database test]
  (when (<= (count (:nodes test)) minimum-voters)
    (throw (ex-info "Membership nemesis requires more than three nodes"
                    {:minimum-voters minimum-voters
                     :nodes (:nodes test)})))
  {:name :membership
   :interval change-seconds
   :nemesis (membership-nemesis database)
   :generator (membership-generator)
   :final-generator {:type :info
                     :f :restore-membership}
   :checker (openraft-checker/reject-checker-exceptions
             (coverage-checker))})

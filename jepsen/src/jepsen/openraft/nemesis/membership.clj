(ns jepsen.openraft.nemesis.membership
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]
             [util :as util]]
            [jepsen.openraft [client :as client]
             [cluster :as cluster]
             [db :as openraft-db]]
            [jepsen.openraft.quorum :as quorum]))

(def change-seconds 10)
(def minimum-voters 3)
(def learner-wait-timeout-ms 60000)

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
    (or (#{:request-timeout :transport-error :invalid-json} kind)
        (and (= :http-error kind)
             (or (nil? status)
                 (<= 500 status 599))))))

(defn- leader-routing-error? [e]
  (let [{:keys [kind error]} (ex-data e)]
    (or (= :unreachable kind)
        (contains? error :ForwardToLeader))))

(defn- membership-change-in-progress? [e]
  (contains? (get-in (ex-data e)
                     [:error :ChangeMembershipError])
             :InProgress))

(defn- await-reachable-learner! [test node]
  (try
    (util/await-fn
     #(cluster/node-metrics! test node)
     {:log-message (str "Waiting for OpenRaft learner " node)
      :timeout learner-wait-timeout-ms})
    (catch InterruptedException e
      (.interrupt (Thread/currentThread))
      (throw e))
    (catch Exception e
      (throw (ex-info "Existing OpenRaft learner is unreachable"
                      {:kind :learner-unreachable
                       :node node}
                      e)))))

(defn- request-membership-change!
  [test leader-endpoint target-voters]
  (try
    (request-leader!
     test
     leader-endpoint
     #(client/change-membership! % (voter-ids test target-voters)))
    :completed
    (catch Exception e
      (cond
        (leader-routing-error? e)
        :no-supported-leader

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
        (throw e)))))

(defn- complete-joint-membership!
  [test {:keys [leader effective-voter-configs]}]
  (let [target-voters (peek effective-voter-configs)
        leader-endpoint (atom (client/api-endpoint test leader))]
    (info "Completing an existing joint OpenRaft membership"
          {:configs effective-voter-configs
           :target-voters target-voters})
    (request-membership-change! test leader-endpoint target-voters)
    (cluster/await-stable-membership! test target-voters)))

(defn- stable-membership! [test]
  (let [{:keys [stable?] :as status}
        (cluster/await-committed-membership! test)]
    (if stable?
      status
      (complete-joint-membership! test status))))

(defn- change-membership-and-await!
  [test leader-endpoint target-voters]
  (when (= :in-progress
           (request-membership-change! test leader-endpoint target-voters))
    (let [status (stable-membership! test)]
      (when-not (= target-voters (:voters status))
        (request-membership-change!
         test
         leader-endpoint
         target-voters))))
  (try
    (cluster/await-stable-membership! test target-voters)
    (catch Exception e
      (when-not (= :timeout (:type (ex-data e)))
        (throw e))
      (let [status (stable-membership! test)]
        (if (= target-voters (:voters status))
          status
          (throw e))))))

(defn- await-observed-learner! [test node]
  (util/await-fn
   #(let [status (cluster/membership-status test)]
      (if (and (:stable? status)
               (contains? (:learners status) node))
        status
        (throw (ex-info "OpenRaft learner is not committed yet"
                        {:node node
                         :status status}))))
   {:log-message (str "Waiting for OpenRaft learner " node
                      " to appear in membership")
    :timeout learner-wait-timeout-ms}))

(defn- request-add-learner!
  [test leader-endpoint node node-id api-addr raft-addr]
  (try
    (request-leader!
     test
     leader-endpoint
     #(client/add-learner! % node-id api-addr raft-addr))
    :completed
    (catch Exception e
      (cond
        (leader-routing-error? e)
        :no-supported-leader

        (ambiguous-request-error? e)
        (do
          (info "OpenRaft learner addition result is indeterminate"
                {:node node
                 :error (ex-message e)})
          :indeterminate)

        :else
        (throw e)))))

(defn- add-learner-and-confirm!
  [test leader-endpoint node node-id api-addr raft-addr]
  (let [result (request-add-learner! test
                                     leader-endpoint
                                     node
                                     node-id
                                     api-addr
                                     raft-addr)]
    (when (= :indeterminate result)
      (await-observed-learner! test node))
    result))

(defn- grow! [database test]
  (let [{:keys [leader voters learners non-members metrics]}
        (stable-membership! test)
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
        (add-learner-and-confirm!
         test
         leader-endpoint
         node
         node-id
         api-addr
         raft-addr)

        (info "Growing OpenRaft membership"
              {:node node
               :before voters
               :after target-voters})

        (let [final-status
              (change-membership-and-await!
               test
               leader-endpoint
               target-voters)]
          {:node node
           :source source
           :leader (:leader final-status)
           :before voters
           :after (:voters final-status)})))))

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
      (dissoc :target)
      (assoc :leader leader
             :after (:target pending))))

(defn- retain-pending-change!
  [pending-change pending status]
  (reset! pending-change pending)
  (assoc pending :status status))

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
        (retain-pending-change! pending-change pending learner-result)
        (do
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
              (retain-pending-change! pending-change pending result))))
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
    {:status result
     :target target-voters
     :voter-configs (:effective-voter-configs status)}))

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

(defn- complete-pending-removal-and-await!
  [database pending-change test]
  (when-let [{:keys [node target] :as pending} @pending-change]
    (let [status (stable-membership! test)
          final-status (if (= target (:voters status))
                         status
                         (change-membership-and-await!
                          test
                          (atom (client/api-endpoint test (:leader status)))
                          target))]
      (openraft-db/stop-and-wipe-node! database test node)
      (reset! pending-change nil)
      (-> pending
          (dissoc :target)
          (assoc :leader (:leader final-status)
                 :after target)))))

(defn- confirm-pending-grow!
  [pending-change test]
  (let [{:keys [target] :as pending} @pending-change
        status (stable-membership! test)]
    (reset! pending-change nil)
    (when (= target (:voters status))
      (-> pending
          (dissoc :target)
          (assoc :leader (:leader status)
                 :after target)))))

(defn- complete-pending-change-and-await!
  [database pending-change test]
  (case (:change @pending-change)
    :grow (confirm-pending-grow! pending-change test)
    :shrink (complete-pending-removal-and-await!
             database
             pending-change
             test)
    nil nil))

(defn- restore-membership! [database pending-change test]
  (let [resolved-change (complete-pending-change-and-await!
                         database
                         pending-change
                         test)
        target-voters (set (:nodes test))]
    (loop [status (stable-membership! test)]
      (if (= target-voters (:voters status))
        (cond-> {:leader (:leader status)
                 :voters (:voters status)}
          resolved-change
          (assoc :resolved-change resolved-change))
        (let [result (grow! database test)]
          (when (keyword? result)
            (throw (ex-info "Unable to restore OpenRaft membership"
                            {:result result
                             :status status
                             :target-voters target-voters})))
          (recur (stable-membership! test)))))))

(defrecord MembershipNemesis [database pending-change]
  nemesis/Nemesis
  (setup! [_ _test]
    (MembershipNemesis. database (atom nil)))

  (invoke! [_ test op]
    (case (:f op)
      :grow
      (assoc op :value (request-membership-operation!
                        database
                        pending-change
                        test
                        :grow))

      :shrink
      (assoc op :value (request-membership-operation!
                        database
                        pending-change
                        test
                        :shrink))

      :restore-membership
      (assoc op :value (restore-membership!
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
                                     (when (and (map? change)
                                                (required-changes
                                                 (:change change))
                                                (contains? change :before)
                                                (contains? change :after)
                                                (not= (:before change)
                                                      (:after change)))
                                       (:change change))))
                                  set)
            missing-changes (remove observed-changes
                                    required-changes)
            final-operation (peek membership-history)
            restored? (and (= :restore-membership (:f final-operation))
                           (= expected-voters
                              (get-in final-operation
                                      [:value :voters])))
            final-recovery (->> history
                                (filter #(= :await-recovery (:f %)))
                                last)
            recovered? (boolean (get-in final-recovery
                                        [:value :leader]))]
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
   :checker (coverage-checker)})

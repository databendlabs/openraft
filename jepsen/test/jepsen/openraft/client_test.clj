(ns jepsen.openraft.client-test
  (:require [cheshire.core :as json]
            [cheshire.parse :as json-parse]
            [clojure.test :refer [deftest is testing]]
            [jepsen.openraft.client :as client]))

(defn- raw-http-response [body]
  {:method "POST"
   :uri "http://n1:21001/write"
   :status 200
   :body body})

(defn- http-response [body]
  (raw-http-response (json/generate-string body)))

(defn- caught [f]
  (try
    (f)
    nil
    (catch Throwable throwable
      throwable)))

(defn- throwing-http-client [throwable]
  (proxy [java.net.http.HttpClient] []
    (send [_request _handler]
      (throw throwable))))

(defn- responding-http-client [status body]
  (let [response (proxy [java.net.http.HttpResponse] []
                   (statusCode [] status)
                   (body [] body)
                   (request [] nil)
                   (previousResponse [] nil)
                   (headers [] nil)
                   (sslSession [] nil)
                   (uri [] nil)
                   (version [] nil))]
    (proxy [java.net.http.HttpClient] []
      (send [_request _handler]
        response))))

(defn- forward-error
  ([]
   (forward-error nil))
  ([endpoint]
   (ex-info "forward"
            {:kind :openraft-error
             :error {:ForwardToLeader
                     {:leader_id (when endpoint 2)
                      :leader_node (when endpoint
                                     {:data endpoint})}}})))

(deftest follows-leader
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n1:21001" endpoint)
                     (throw (forward-error "n2:21001"))
                     :ok)))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001"] @attempts))
    (is (= "n2:21001" @leader))))

(deftest searches-nodes-when-leader-is-unknown
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n3:21001" endpoint)
                     :ok
                     (throw (forward-error)))))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001" "n3:21001"] @attempts))
    (is (= "n3:21001" @leader))))

(deftest retries-unreachable-endpoint
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n1:21001" endpoint)
                     (throw (ex-info "unreachable" {:kind :unreachable}))
                     :ok)))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001"] @attempts)
        "a connection failure proves nothing was applied and is retried")
    (is (= "n2:21001" @leader))))

(deftest ignores-empty-forward-target
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n1:21001" endpoint)
                     (throw (forward-error ""))
                     :ok)))]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001"] @attempts)
        "an empty forward target is skipped in favor of a known endpoint")
    (is (= "n2:21001" @leader))))

(deftest does-not-retry-request-timeout
  (let [leader (atom "n1:21001")
        attempts (atom 0)
        error (try
                (client/with-leader!
                  leader
                  ["n1:21001" "n2:21001" "n3:21001"]
                  (fn [_]
                    (swap! attempts inc)
                    (throw (ex-info "timeout"
                                    {:kind :request-timeout}))))
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is (= :request-timeout (:kind (ex-data error)))
        "request timeouts must reach the workload error classifier")
    (is (= 1 @attempts)
        "request timeouts must not be retried because a mutation may have committed")
    (is (= "n2:21001" @leader)
        "the next operation should avoid the endpoint which timed out")))

(deftest retries-read-timeout-on-another-node
  (let [leader (atom "n1:21001")
        attempts (atom [])
        result (client/with-leader!
                 leader
                 ["n1:21001" "n2:21001" "n3:21001"]
                 (fn [endpoint]
                   (swap! attempts conj endpoint)
                   (if (= "n1:21001" endpoint)
                     (throw (ex-info "timeout"
                                     {:kind :request-timeout}))
                     :ok))
                 client/retryable-read-error?)]
    (is (= :ok result))
    (is (= ["n1:21001" "n2:21001"] @attempts))
    (is (= "n2:21001" @leader))))

(deftest does-not-cache-an-unverified-leader
  (let [leader (atom "n1:21001")
        attempts (atom [])
        error (try
                (client/with-leader!
                  leader
                  ["n1:21001" "n2:21001" "n3:21001"]
                  (fn [endpoint]
                    (swap! attempts conj endpoint)
                    (throw (forward-error))))
                nil
                (catch clojure.lang.ExceptionInfo e
                  e))]
    (is error)
    (is (= ["n1:21001" "n2:21001" "n3:21001"] @attempts))
    (is (= "n1:21001" @leader)
        "a retry candidate becomes the cached leader only after it succeeds")))

(deftest accepts-valid-workload-responses
  (testing "write returns a versioned value, including an empty string"
    (let [versioned {:value "" :version 0}]
      (with-redefs [client/post! (fn [& _]
                                   (http-response
                                    {:Ok {:data {:value versioned}}}))]
        (is (= versioned (client/write! "n1:21001" "key" ""))))))

  (testing "write accepts the largest u64 version"
    (let [versioned {:value "value" :version 18446744073709551615N}]
      (with-redefs [client/post! (fn [& _]
                                   (http-response
                                    {:Ok {:data {:value versioned}}}))]
        (is (= versioned
               (client/write! "n1:21001" "key" "value"))))))

  (testing "CAS accepts an explicit nil value as a version mismatch"
    (with-redefs [client/post! (fn [& _]
                                 (http-response
                                  {:Ok {:data {:value nil}}}))]
      (is (nil? (client/cas! "n1:21001" "key" 1 "value")))))

  (testing "CAS accepts a matching non-nil value"
    (let [versioned {:value "new" :version 2}]
      (with-redefs [client/post! (fn [& _]
                                   (http-response
                                    {:Ok {:data {:value versioned}}}))]
        (is (= versioned
               (client/cas! "n1:21001" "key" 1 "new"))))))

  (testing "read accepts an explicit nil value for a missing key"
    (with-redefs [client/post! (fn [& _]
                                 (http-response {:Ok {:value nil}}))]
      (is (nil? (client/linearizable-read! "n1:21001" "key")))))

  (testing "additive response fields do not invalidate the Ok arm"
    (let [versioned {:value "value" :version 1}]
      (with-redefs [client/post! (fn [& _]
                                   (http-response
                                    {:Ok {:data {:value versioned}}
                                     :trace "diagnostic"}))]
        (is (= versioned
               (client/write! "n1:21001" "key" "value")))))))

(deftest preserves-known-application-errors
  (let [error {:QuorumNotEnough {:cluster "n1,n2,n3"
                                 :got ["n1"]}}
        response (http-response {:Err error})
        thrown (with-redefs [client/post! (fn [& _] response)]
                 (caught #(client/linearizable-read! "n1:21001" "key")))]
    (is (= :openraft-error (:kind (ex-data thrown))))
    (is (= error (:error (ex-data thrown))))
    (is (= (select-keys response [:method :uri :status])
           (select-keys (:response (ex-data thrown)) [:method :uri :status])))
    (is (= (count (:body response))
           (get-in (ex-data thrown) [:response :body-character-count])))
    (is (= (:body response)
           (get-in (ex-data thrown) [:response :body-preview])))))

(deftest rejects-invalid-application-response-unions
  (doseq [[label body reason]
          [["missing arm" {} :missing-response-arm]
           ["ambiguous arms"
            {:Ok {:data {:value {:value "value" :version 1}}}
             :Err {:ForwardToLeader {}}}
            :ambiguous-response-arms]
           ["unknown arm" {:Result {}} :unknown-response-arm]
           ["non-object union" [] :response-union-not-map]]]
    (testing label
      (let [response (http-response body)
            thrown (with-redefs [client/post! (fn [& _] response)]
                     (caught #(client/write! "n1:21001" "key" "value")))
            data (ex-data thrown)]
        (is (= :invalid-response (:kind data)))
        (is (= reason (:reason data)))
        (is (= (when (map? body)
                 (count (filter #{:Ok :Err} (keys body))))
               (:response-arm-count data)))
        (is (= (select-keys response [:method :uri :status])
               (select-keys (:response data) [:method :uri :status])))
        (is (= (count (:body response))
               (get-in data [:response :body-character-count])))
        (is (= (:body response)
               (get-in data [:response :body-preview])))
        (is (not (contains? data :decoded-response))))))

  (testing "Err contains exactly one error variant"
    (let [body {:Err {:ForwardToLeader {}
                      :QuorumNotEnough {}}}
          response (http-response body)
          thrown (with-redefs [client/post! (fn [& _] response)]
                   (caught #(client/write! "n1:21001" "key" "value")))
          data (ex-data thrown)]
      (is (= :invalid-response (:kind data)))
      (is (= :invalid-error-union (:reason data)))
      (is (= 2 (:error-arm-count data)))
      (is (= (:body response)
             (get-in data [:response :body-preview])))))

  (testing "a non-object root is rejected without materializing it"
    (let [decoded? (atom false)
          response (http-response [])
          thrown (with-redefs [client/post! (fn [& _] response)
                               json-parse/parse-strict
                               (fn [& _]
                                 (reset! decoded? true)
                                 (throw (RuntimeException.
                                         "unexpected full decode")))]
                   (caught #(client/write! "n1:21001" "key" "value")))]
      (is (= :invalid-response (:kind (ex-data thrown))))
      (is (= :response-union-not-map (:reason (ex-data thrown))))
      (is (false? @decoded?)))))

(deftest rejects-incomplete-or-ambiguous-json-documents
  (let [valid-ok
        "{\"Ok\":{\"data\":{\"value\":{\"value\":\"value\",\"version\":1}}}}"]
    (doseq [[label body]
            [["duplicate top-level key"
              (str "{\"Ok\":{\"data\":{\"value\":"
                   "{\"value\":\"value\",\"version\":1}}},"
                   "\"Ok\":{\"data\":{\"value\":"
                   "{\"value\":\"value\",\"version\":2}}}}")]
             ["duplicate nested key"
              "{\"Err\":{\"ForwardToLeader\":{},\"ForwardToLeader\":[]}}"]
             ["second root document"
              (str valid-ok " {\"Err\":{\"ForwardToLeader\":{}}}")]
             ["trailing garbage" (str valid-ok " garbage")]
             ["empty body" ""]
             ["whitespace-only body" " \n\t"]]]
      (testing label
        (let [response (raw-http-response body)
              thrown (with-redefs [client/post! (fn [& _] response)]
                       (caught #(client/write! "n1:21001" "key" "value")))]
          (is (= :invalid-json (:kind (ex-data thrown))))
          (is (= (select-keys response [:method :uri :status])
                 (select-keys (:response (ex-data thrown))
                              [:method :uri :status])))
          (is (= (count body)
                 (get-in (ex-data thrown)
                         [:response :body-character-count]))))))

    (testing "trailing whitespace is part of one complete document"
      (let [response (raw-http-response (str valid-ok " \n\t"))]
        (with-redefs [client/post! (fn [& _] response)]
          (is (= {:value "value" :version 1}
                 (client/write! "n1:21001" "key" "value"))))))))

(deftest validates-modeled-error-payload-containers
  (testing "an empty ForwardToLeader object remains modeled"
    (let [error {:ForwardToLeader {}}
          thrown (with-redefs [client/post! (fn [& _]
                                              (http-response {:Err error}))]
                   (caught #(client/write! "n1:21001" "key" "value")))]
      (is (= :openraft-error (:kind (ex-data thrown))))
      (is (= error (:error (ex-data thrown))))))

  (doseq [[variant payload]
          [[:ForwardToLeader "not-an-object"]
           [:QuorumNotEnough ["not" "an" "object"]]]]
    (testing (name variant)
      (let [thrown (with-redefs [client/post! (fn [& _]
                                                (http-response
                                                 {:Err {variant payload}}))]
                     (caught #(client/write! "n1:21001" "key" "value")))
            data (ex-data thrown)]
        (is (= :invalid-response (:kind data)))
        (is (= :invalid-error-payload (:reason data)))
        (is (= variant (:error-variant data)))))))

(deftest bounds-response-error-evidence
  (testing "an invalid response"
    (let [body {:Unknown (apply str (repeat 2048 "x"))}
          response (http-response body)
          thrown (with-redefs [client/post! (fn [& _] response)]
                   (caught #(client/write! "n1:21001" "key" "value")))
          data (ex-data thrown)
          evidence (:response data)]
      (is (= :invalid-response (:kind data)))
      (is (= :unknown-response-arm (:reason data)))
      (is (= (count (:body response)) (:body-character-count evidence)))
      (is (= 1024 (count (:body-preview evidence))))
      (is (not (contains? data :decoded-response)))
      (is (not (contains? data :payload)))))

  (testing "an unmodeled OpenRaft error"
    (let [body {:FutureError (apply str (repeat 2048 "x"))}
          response (http-response {:Err body})
          thrown (with-redefs [client/post! (fn [& _] response)]
                   (caught #(client/write! "n1:21001" "key" "value")))
          evidence (:response (ex-data thrown))]
      (is (= :openraft-error (:kind (ex-data thrown))))
      (is (= (count (:body response)) (:body-character-count evidence)))
      (is (= 1024 (count (:body-preview evidence))))
      (is (not (contains? evidence :body)))))

  (testing "a non-200 response"
    (let [body (apply str (repeat 2048 "x"))
          thrown (with-redefs-fn
                   {(ns-resolve 'jepsen.openraft.client 'http-client)
                    (responding-http-client 500 body)}
                   #(caught (fn [] (client/post! "n1:21001" "/write" {}))))
          data (ex-data thrown)]
      (is (= :http-error (:kind data)))
      (is (= 500 (:status data)))
      (is (= (count body) (:body-character-count data)))
      (is (= 1024 (count (:body-preview data))))
      (is (not (contains? data :body))))))

(deftest rejects-malformed-workload-payloads
  (doseq [[label invoke body reason path]
          [["write nil"
            #(client/write! "n1:21001" "key" "value")
            {:Ok {:data {:value nil}}}
            :nil-payload-field
            [:data :value]]
           ["write missing data"
            #(client/write! "n1:21001" "key" "value")
            {:Ok {}}
            :missing-payload-field
            [:data :value]]
           ["CAS missing value"
            #(client/cas! "n1:21001" "key" 1 "value")
            {:Ok {:data {}}}
            :missing-payload-field
            [:data :value]]
           ["read malformed value"
            #(client/linearizable-read! "n1:21001" "key")
            {:Ok {:value {:value 1 :version 2}}}
            :invalid-versioned-value
            [:value]]
           ["write version exceeds u64"
            #(client/write! "n1:21001" "key" "value")
            {:Ok {:data {:value {:value "value"
                                 :version 18446744073709551616N}}}}
            :invalid-versioned-value
            [:data :value]]
           ["write returns a different logical value"
            #(client/write! "n1:21001" "key" "value")
            {:Ok {:data {:value {:value "other" :version 1}}}}
            :unexpected-payload-value
            [:data :value :value]]
           ["CAS returns a different logical value"
            #(client/cas! "n1:21001" "key" 1 "new")
            {:Ok {:data {:value {:value "other" :version 2}}}}
            :unexpected-payload-value
            [:data :value :value]]
           ["CAS returns a non-increasing version"
            #(client/cas! "n1:21001" "key" 2 "new")
            {:Ok {:data {:value {:value "new" :version 2}}}}
            :non-increasing-cas-version
            [:data :value :version]]]]
    (testing label
      (let [response (http-response body)
            thrown (with-redefs [client/post! (fn [& _] response)]
                     (caught invoke))
            data (ex-data thrown)]
        (is (= :invalid-response (:kind data)))
        (is (= reason (:reason data)))
        (is (= path (:payload-path data)))
        (is (= (count (:body response))
               (get-in data [:response :body-character-count])))
        (is (= (:body response)
               (get-in data [:response :body-preview])))
        (is (not (contains? data :payload)))))))

(deftest distinguishes-invalid-json-from-parser-failures
  (testing "malformed JSON is a SUT response error"
    (let [response (assoc (http-response {}) :body "{")
          thrown (with-redefs [client/post! (fn [& _] response)]
                   (caught #(client/write! "n1:21001" "key" "value")))]
      (is (= :invalid-json (:kind (ex-data thrown))))
      (is (= (select-keys response [:method :uri :status])
             (select-keys (:response (ex-data thrown))
                          [:method :uri :status])))
      (is (= 1 (get-in (ex-data thrown)
                       [:response :body-character-count])))
      (is (= "{" (get-in (ex-data thrown)
                         [:response :body-preview])))))

  (testing "an unknown parser exception escapes unchanged"
    (let [throwable (RuntimeException. "parser bug")
          response (http-response
                    {:Ok {:data {:value {:value "value" :version 1}}}})
          thrown (with-redefs [client/post! (fn [& _] response)
                               json-parse/parse-strict (fn [& _]
                                                         (throw throwable))]
                   (caught #(client/write! "n1:21001" "key" "value")))]
      (is (identical? throwable thrown)))))

(deftest propagates-http-client-interruptions
  (doseq [[label throwable]
          [["InterruptedException" (InterruptedException. "interrupted")]
           ["InterruptedIOException"
            (java.io.InterruptedIOException. "interrupted")]
           ["ClosedByInterruptException"
            (java.nio.channels.ClosedByInterruptException.)]]]
    (testing label
      (Thread/interrupted)
      (let [fake-client (throwing-http-client throwable)
            thrown (with-redefs-fn
                     {(ns-resolve 'jepsen.openraft.client 'http-client)
                      fake-client}
                     #(caught
                       (fn []
                         (client/post! "n1:21001" "/write" {}))))
            interrupted (Thread/interrupted)]
        (is (identical? throwable thrown))
        (is interrupted))))

  (testing "ordinary I/O remains a transport error"
    (let [throwable (java.io.IOException. "broken pipe")
          fake-client (throwing-http-client throwable)
          thrown (with-redefs-fn
                   {(ns-resolve 'jepsen.openraft.client 'http-client)
                    fake-client}
                   #(caught
                     (fn []
                       (client/post! "n1:21001" "/write" {}))))]
      (is (= :transport-error (:kind (ex-data thrown))))
      (is (identical? throwable (ex-cause thrown))))))
